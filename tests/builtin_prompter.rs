// SPDX-License-Identifier: GPL-3.0-only

//! The built-in prompter against an isolated oo7-daemon with no desktop
//! prompter, as on COSMIC. Remove with the `builtin-prompter` feature.

#![cfg(feature = "builtin-prompter")]
#![allow(clippy::pedantic)]

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use keybox::backend::prompter::{PromptKind, Prompter, Requests};
use keybox::backend::{Client, CollectionInfo, Error, PromptOutcome};
use support::TestEnv;
use zbus::zvariant::{ObjectPath, Value};
use zeroize::Zeroizing;

const PROMPTER_NAME: &str = "org.gnome.keyring.SystemPrompter";

macro_rules! require_tools {
    () => {
        if !support::tools_available() {
            return;
        }
    };
}

async fn collection(client: &Client, label: &str) -> CollectionInfo {
    client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.label.as_deref() == Some(label))
        .unwrap()
}

async fn start(client: &Client) -> (Prompter, Requests) {
    Prompter::start(client)
        .await
        .unwrap()
        .expect("no other prompter on the private bus")
}

/// Records `PromptReady` calls on the bus, to check what crosses it.
async fn prompt_ready_recorder(env: &TestEnv) -> Arc<Mutex<Vec<Vec<u8>>>> {
    let conn = env.raw_connection().await;
    let rules = vec![
        "type='method_call',interface='org.gnome.keyring.internal.Prompter.Callback',member='PromptReady'",
    ];
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus.Monitoring"),
        "BecomeMonitor",
        &(rules, 0u32),
    )
    .await
    .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let mut stream = zbus::MessageStream::from(&conn);
    tokio::spawn(async move {
        let _keep = conn;
        while let Some(Ok(msg)) = stream.next().await {
            recorder.lock().unwrap().push(msg.data().to_vec());
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn unlocks_with_retry_and_never_sends_plaintext() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let recorded = prompt_ready_recorder(&env).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;
    assert_eq!(work.locked, Some(true));

    let (prompter, mut requests) = start(&client).await;
    let answers = tokio::spawn(async move {
        let first = requests.next().await.unwrap();
        assert_eq!((first.kind, first.retry), (PromptKind::Unlock, false));
        first.submit(Zeroizing::new("kbx-wrong-password".into()));
        let second = requests.next().await.unwrap();
        assert_eq!((second.kind, second.retry), (PromptKind::Unlock, true));
        second.submit(Zeroizing::new(support::WORK_PASSWORD.into()));
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        client.unlock(std::slice::from_ref(&work.id), Some(prompter.window_id())),
    )
    .await
    .expect("unlock completes")
    .unwrap();
    assert_eq!(outcome, PromptOutcome::Completed);
    answers.await.unwrap();
    assert_eq!(collection(&client, "Work").await.locked, Some(false));

    let recorded = recorded.lock().unwrap();
    assert!(recorded.len() >= 2, "monitor saw the replies");
    for body in recorded.iter() {
        assert!(!support::contains_bytes(body, b"kbx-wrong-password"));
        assert!(!support::contains_bytes(
            body,
            support::WORK_PASSWORD.as_bytes()
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_dismisses_the_prompt() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;

    let (prompter, mut requests) = start(&client).await;
    tokio::spawn(async move { requests.next().await.unwrap().cancel() });
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        client.unlock(std::slice::from_ref(&work.id), Some(prompter.window_id())),
    )
    .await
    .expect("unlock completes");
    assert_eq!(result.unwrap_err(), Error::PromptDismissed);
    assert_eq!(collection(&client, "Work").await.locked, Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn leaves_other_applications_prompts_alone() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let other_app = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;
    let (prompter, mut requests) = start(&client).await;

    // Another application's unlock gets no answer from Keybox.
    let pending = tokio::time::timeout(
        Duration::from_millis(1500),
        other_app.unlock(&[collection(&other_app, "Work").await.id], None),
    )
    .await;
    assert!(
        pending.is_err(),
        "the other application is still waiting: {pending:?}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), requests.next())
            .await
            .is_err(),
        "no password was requested for it"
    );

    // Keybox's own request still works.
    tokio::spawn(async move {
        let request = requests.next().await.unwrap();
        request.submit(Zeroizing::new(support::WORK_PASSWORD.into()));
    });
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        client.unlock(std::slice::from_ref(&work.id), Some(prompter.window_id())),
    )
    .await
    .expect("unlock completes")
    .unwrap();
    assert_eq!(outcome, PromptOutcome::Completed);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_keyring_service_may_call_it() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let (prompter, mut requests) = start(&client).await;

    let intruder = env.raw_connection().await;
    let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
    properties.insert("caller-window", Value::new(prompter.window_id()));
    let result = intruder
        .call_method(
            Some(PROMPTER_NAME),
            "/org/gnome/keyring/Prompter",
            Some("org.gnome.keyring.internal.Prompter"),
            "PerformPrompt",
            &(
                ObjectPath::try_from("/kbx/fake").unwrap(),
                "password",
                properties,
                "[sx-aes-1]\npublic=AQI=\n",
            ),
        )
        .await;
    assert!(
        matches!(&result, Err(zbus::Error::MethodError(name, ..)) if name.as_str() == "org.freedesktop.DBus.Error.AccessDenied"),
        "{result:?}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), requests.next())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn steps_aside_for_a_real_prompter() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();

    // Running prompter.
    let real = env.raw_connection().await;
    real.request_name(PROMPTER_NAME).await.unwrap();
    assert!(Prompter::start(&client).await.unwrap().is_none());
    real.release_name(PROMPTER_NAME).await.unwrap();

    // Installed but not running (D-Bus activatable).
    std::fs::write(
        env.dir()
            .join("services/org.gnome.keyring.SystemPrompter.service"),
        "[D-BUS Service]\nName=org.gnome.keyring.SystemPrompter\nExec=/bin/false\n",
    )
    .unwrap();
    real.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ReloadConfig",
        &(),
    )
    .await
    .unwrap();
    assert!(Prompter::start(&client).await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn releases_the_name_when_dropped() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let watcher = env.raw_connection().await;
    let dbus = zbus::fdo::DBusProxy::new(&watcher).await.unwrap();
    let name = zbus::names::BusName::try_from(PROMPTER_NAME).unwrap();

    let started = start(&client).await;
    assert!(dbus.name_has_owner(name.clone()).await.unwrap());
    drop(started);
    for _ in 0..50 {
        if !dbus.name_has_owner(name.clone()).await.unwrap() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("prompter name still owned after drop");
}

#[tokio::test(flavor = "multi_thread")]
async fn asks_for_a_new_keyrings_password() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let (prompter, mut requests) = start(&client).await;
    tokio::spawn(async move {
        let request = requests.next().await.unwrap();
        assert_eq!(
            (request.kind, request.retry),
            (PromptKind::NewPassword, false)
        );
        request.submit(Zeroizing::new("kbx-new-keyring-password".into()));
    });
    let created = tokio::time::timeout(
        Duration::from_secs(10),
        client.create_collection("KBX New", Some(prompter.window_id())),
    )
    .await
    .expect("creation completes")
    .unwrap();
    let new = collection(&client, "KBX New").await;
    assert_eq!(new.id, created);
    assert_eq!(new.locked, Some(false));
}
