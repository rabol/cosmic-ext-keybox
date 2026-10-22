// SPDX-License-Identifier: GPL-3.0-only

//! Backend integration tests against an isolated GNOME Keyring.
//!
//! Uses an installed `gnome-keyring-daemon`, or an unpacked package root in
//! `KEYBOX_TEST_GNOME_KEYRING_ROOT`. Skipped otherwise. Like the oo7 tests,
//! everything runs on a private bus with temporary data directories.

#![allow(clippy::pedantic)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use keybox::backend::{
    Client, CollectionInfo, Error, ItemInfo, PromptOutcome, Provider, ServiceEvent,
};
use keybox::secret::SecretValue;
use support::TestEnv;

macro_rules! env_or_skip {
    () => {
        match TestEnv::gnome_keyring().await {
            Some(env) => env,
            None => return,
        }
    };
}

async fn login(client: &Client) -> CollectionInfo {
    client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.is_default == Some(true))
        .expect("default collection")
}

async fn by_kind(client: &Client, collection: &CollectionInfo, kind: &str) -> Vec<ItemInfo> {
    client
        .items(&collection.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|i| {
            i.attributes
                .as_ref()
                .and_then(|a| a.get("kind"))
                .map(String::as_str)
                == Some(kind)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_with_encryption_and_detects_gnome_keyring() {
    let env = env_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    assert_eq!(client.provider(), Some(Provider::GnomeKeyring));
    assert!(client.is_encrypted());
    assert_eq!(login(&client).await.locked, Some(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn browsing_never_retrieves_secrets() {
    let env = env_or_skip!();
    let secret_calls = support::secret_call_counter(&env).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let _events = client.events().await.unwrap();
    for c in client.collections().await.unwrap() {
        client.items(&c.id).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(secret_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_and_secret_classification() {
    let env = env_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = login(&client).await;

    let dups = by_kind(&client, &login, "dup").await;
    assert_eq!(dups.len(), 2);
    assert_ne!(dups[0].id, dups[1].id);
    let large = &by_kind(&client, &login, "large").await[0];
    assert_eq!(
        large.label.as_ref().unwrap().len(),
        support::LARGE_LABEL_LEN
    );

    let secret = |kind: &'static str| {
        let client = client.clone();
        let login = login.clone();
        async move {
            let item = by_kind(&client, &login, kind).await.remove(0);
            client.secret(&item.id).await.unwrap()
        }
    };
    let SecretValue::Text(t) = secret("alpha").await.value else {
        panic!()
    };
    assert_eq!(t.expose(), support::SECRET_ALPHA);
    let SecretValue::Text(t) = secret("empty").await.value else {
        panic!()
    };
    assert!(t.is_empty());
    let SecretValue::TextWithControl(t) = secret("control").await.value else {
        panic!()
    };
    assert_eq!(t.expose(), support::SECRET_CONTROL);
    // GNOME Keyring reports text/plain even for binary data; the content
    // decides.
    let binary = secret("binary").await;
    assert!(matches!(binary.value, SecretValue::Binary { len: 4 }));
    // Stored with "text/plain; charset=utf8".
    let SecretValue::Text(t) = secret("charset").await.value else {
        panic!()
    };
    assert_eq!(t.expose(), "KBX-FIXTURE-CHARSET");
}

#[tokio::test(flavor = "multi_thread")]
async fn lock_keeps_metadata_but_refuses_secrets() {
    let env = env_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let mut events = Box::pin(client.events().await.unwrap());
    let login = login(&client).await;

    assert_eq!(
        client
            .lock(std::slice::from_ref(&login.id), None)
            .await
            .unwrap(),
        PromptOutcome::Done
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(ServiceEvent::CollectionChanged(id))) if id == login.id => break,
            Ok(Some(_)) => {}
            other => panic!("no CollectionChanged: {other:?}"),
        }
    }
    let items = client.items(&login.id).await.unwrap();
    assert!(items.iter().all(|i| i.locked == Some(true)));
    // Unlike oo7, GNOME Keyring keeps labels visible while locked.
    assert!(
        items
            .iter()
            .any(|i| i.label.as_deref() == Some("KBX Alpha"))
    );
    assert_eq!(
        client.secret(&items[0].id).await.unwrap_err(),
        Error::Locked
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unlock_without_prompter_is_dismissed() {
    let env = env_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = login(&client).await;
    client
        .lock(std::slice::from_ref(&login.id), None)
        .await
        .unwrap();
    // No org.gnome.keyring.SystemPrompter on the private bus: GNOME Keyring
    // reports a dismissed prompt instead of waiting (oo7 waits).
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.unlock(std::slice::from_ref(&login.id), None),
    )
    .await
    .expect("should not hang");
    assert_eq!(result.unwrap_err(), Error::PromptDismissed);
}

#[tokio::test(flavor = "multi_thread")]
async fn service_restart_invalidates_identities() {
    let mut env = env_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let mut events = Box::pin(client.events().await.unwrap());
    let login = login(&client).await;
    env.stop_oo7().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(ServiceEvent::OwnerChanged)) => break,
            Ok(Some(_)) => {}
            other => panic!("no OwnerChanged: {other:?}"),
        }
    }
    assert!(client.items(&login.id).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn debug_output_never_contains_fixture_data() {
    let env = env_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let mut printed = String::new();
    for c in client.collections().await.unwrap() {
        printed += &format!("{c:?}");
        for item in client.items(&c.id).await.unwrap() {
            printed += &format!("{item:?}{:?}", client.secret(&item.id).await);
        }
    }
    for needle in ["KBX-FIXTURE", "KBX Alpha", "alice", "example.invalid"] {
        assert!(!printed.contains(needle), "leaked {needle:?}");
    }
}
