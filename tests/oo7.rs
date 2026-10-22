// SPDX-License-Identifier: GPL-3.0-only

//! Backend integration tests against an isolated oo7-daemon.
//!
//! See `support/mod.rs`: each test runs its own private bus and daemon with
//! disposable keyrings. The user's real keyring is never used.

#![allow(clippy::pedantic)]

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use keybox::backend::{
    Client, CollectionInfo, Error, ItemInfo, ObjectId, PromptOutcome, Provider, ServiceEvent,
};
use keybox::secret::SecretValue;
use support::{PromptBehaviour, TestEnv};

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
        .unwrap_or_else(|| panic!("no collection {label}"))
}

async fn items_by_kind(client: &Client, collection: &ObjectId, kind: &str) -> Vec<ItemInfo> {
    client
        .items(collection)
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

async fn item_by_kind(client: &Client, collection: &ObjectId, kind: &str) -> ItemInfo {
    let mut found = items_by_kind(client, collection, kind).await;
    assert_eq!(found.len(), 1, "expected one item of kind {kind}");
    found.remove(0)
}

/// Waits for an event matching `pred`, failing after a few seconds.
async fn expect_event(
    events: &mut (impl Stream<Item = ServiceEvent> + Unpin),
    what: &str,
    pred: impl Fn(&ServiceEvent) -> bool,
) -> ServiceEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(event)) if pred(&event) => return event,
            Ok(Some(_)) => {}
            Ok(None) => panic!("event stream ended while waiting for {what}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn connects_with_encryption_and_detects_oo7() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    assert_eq!(client.provider(), Some(Provider::Oo7));
    assert!(client.is_encrypted());

    let again = Client::connect(&env.bus()).await.unwrap();
    assert_ne!(
        client.generation(),
        again.generation(),
        "each connection gets a new generation"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_keyrings_with_lock_and_default_state() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();

    let login = collection(&client, "Login").await;
    assert_eq!(login.locked, Some(false));
    assert_eq!(login.is_default, Some(true));
    assert!(login.error.is_none());

    let work = collection(&client, "Work").await;
    assert_eq!(work.locked, Some(true));
    assert_eq!(work.is_default, Some(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn browsing_never_retrieves_secrets() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let secret_calls = support::secret_call_counter(&env).await;

    let client = Client::connect(&env.bus()).await.unwrap();
    let _events = client.events().await.unwrap();
    for c in client.collections().await.unwrap() {
        let items = client.items(&c.id).await.unwrap();
        for item in &items {
            client.item(&c.id, &item.id).await.unwrap();
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        secret_calls.load(Ordering::SeqCst),
        0,
        "startup/listing fetched a secret"
    );

    // Positive control: the monitor does see an explicit retrieval.
    let login = collection(&client, "Login").await;
    let alpha = item_by_kind(&client, &login.id, "alpha").await;
    client.secret(&alpha.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(secret_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_edge_cases() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = collection(&client, "Login").await;

    // Duplicate labels stay separate entries.
    let dups = items_by_kind(&client, &login.id, "dup").await;
    assert_eq!(dups.len(), 2);
    assert_eq!(dups[0].label, dups[1].label);
    assert_ne!(dups[0].id, dups[1].id);

    // A missing label is reported as empty, not dropped.
    let nolabel = item_by_kind(&client, &login.id, "nolabel").await;
    assert_eq!(nolabel.label.as_deref(), Some(""));

    // Large metadata arrives intact.
    let large = item_by_kind(&client, &login.id, "large").await;
    assert_eq!(
        large.label.as_ref().unwrap().len(),
        support::LARGE_LABEL_LEN
    );
    let attrs = large.attributes.as_ref().unwrap();
    assert!(attrs.len() > support::LARGE_ATTRIBUTE_COUNT);

    // Markup is returned verbatim; rendering it as text is the UI's job.
    let markup = item_by_kind(&client, &login.id, "markup").await;
    assert!(markup.label.as_ref().unwrap().contains("<b>Markup</b>"));

    // Timestamps are supplied by oo7.
    assert!(markup.created.is_some() && markup.modified.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn retrieves_and_classifies_secrets() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = collection(&client, "Login").await;

    let secret = |kind: &'static str| {
        let client = client.clone();
        let login = login.id.clone();
        async move {
            let item = item_by_kind(&client, &login, kind).await;
            client.secret(&item.id).await.unwrap()
        }
    };

    let alpha = secret("alpha").await;
    assert_eq!(alpha.content_type, "text/plain");
    let SecretValue::Text(text) = &alpha.value else {
        panic!("alpha: {alpha:?}")
    };
    assert_eq!(text.expose(), support::SECRET_ALPHA);

    let empty = secret("empty").await;
    let SecretValue::Text(text) = &empty.value else {
        panic!("empty: {empty:?}")
    };
    assert!(text.is_empty());

    let control = secret("control").await;
    let SecretValue::TextWithControl(text) = &control.value else {
        panic!("control: {control:?}")
    };
    assert_eq!(
        text.expose(),
        support::SECRET_CONTROL,
        "original text preserved for copying"
    );

    let binary = secret("binary").await;
    assert_eq!(binary.content_type, "application/octet-stream");
    assert!(
        matches!(binary.value, SecretValue::Binary { len } if len == support::SECRET_BINARY.len())
    );
    assert!(binary.value.copyable_text().is_none());

    // Declared as binary but valid UTF-8: shown as text.
    let blob_text = secret("blobtext").await;
    let SecretValue::Text(text) = &blob_text.value else {
        panic!("blobtext: {blob_text:?}")
    };
    assert_eq!(text.expose().as_bytes(), support::SECRET_BLOB_TEXT);
}

#[tokio::test(flavor = "multi_thread")]
async fn locked_keyring_hides_metadata_and_refuses_secrets() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;

    let items = client.items(&work.id).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].locked, Some(true));
    assert!(items[0].label.is_none(), "label not disclosed while locked");
    assert!(
        items[0].attributes.is_none(),
        "attributes not disclosed while locked"
    );
    assert_eq!(
        client.secret(&items[0].id).await.unwrap_err(),
        Error::Locked
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lock_updates_state_and_emits_events() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let mut events = Box::pin(client.events().await.unwrap());
    let login = collection(&client, "Login").await;

    assert_eq!(
        client
            .lock(std::slice::from_ref(&login.id), None)
            .await
            .unwrap(),
        PromptOutcome::Done
    );
    let id = login.id.clone();
    expect_event(&mut events, "CollectionChanged(Login)", |e| {
        *e == ServiceEvent::CollectionChanged(id.clone())
    })
    .await;
    assert_eq!(collection(&client, "Login").await.locked, Some(true));

    let alpha_id = client.items(&login.id).await.unwrap()[0].id.clone();
    assert_eq!(client.secret(&alpha_id).await.unwrap_err(), Error::Locked);
}

#[tokio::test(flavor = "multi_thread")]
async fn unlock_uses_provider_prompt_and_passes_window_id() {
    require_tools!();
    let (env, prompter) = TestEnv::oo7_with_prompter(PromptBehaviour::Accept(vec![
        "wrong password".into(),
        support::WORK_PASSWORD.into(),
    ]))
    .await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;

    let outcome = client
        .unlock(
            std::slice::from_ref(&work.id),
            Some("wayland:kbx-test-handle"),
        )
        .await
        .unwrap();
    assert_eq!(outcome, PromptOutcome::Completed);
    assert_eq!(
        *prompter.window_ids.lock().unwrap(),
        ["wayland:kbx-test-handle"]
    );

    assert_eq!(collection(&client, "Work").await.locked, Some(false));
    let item = item_by_kind(&client, &work.id, "work").await;
    assert_eq!(item.label.as_deref(), Some("KBX Work"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_prompt_is_a_normal_outcome() {
    require_tools!();
    let (env, _prompter) = TestEnv::oo7_with_prompter(PromptBehaviour::Reject).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;

    let result = client.unlock(std::slice::from_ref(&work.id), None).await;
    assert_eq!(result.unwrap_err(), Error::PromptDismissed);

    // The client remains usable.
    assert_eq!(collection(&client, "Work").await.locked, Some(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn unanswered_prompt_can_be_abandoned() {
    require_tools!();
    let (env, prompter) = TestEnv::oo7_with_prompter(PromptBehaviour::Ignore).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = collection(&client, "Work").await;

    // Like COSMIC without a prompter: nothing ever answers.
    let pending = tokio::time::timeout(
        Duration::from_millis(1500),
        client.unlock(std::slice::from_ref(&work.id), None),
    )
    .await;
    assert!(pending.is_err(), "unlock should still be waiting");
    assert_eq!(prompter.calls.load(Ordering::SeqCst), 1);

    // Dropping the future dismissed the prompt; the service still responds.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let login = collection(&client, "Login").await;
    assert_eq!(
        client.lock(&[login.id], None).await.unwrap(),
        PromptOutcome::Done
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn external_changes_and_deletion_are_reported() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let mut events = Box::pin(client.events().await.unwrap());
    let login = collection(&client, "Login").await;
    let alpha = item_by_kind(&client, &login.id, "alpha").await;

    // Another application renames the entry.
    let other = env.raw_connection().await;
    let props = zbus::fdo::PropertiesProxy::builder(&other)
        .destination("org.freedesktop.secrets")
        .unwrap()
        .path(alpha.id.path().to_owned())
        .unwrap()
        .build()
        .await
        .unwrap();
    props
        .set(
            "org.freedesktop.Secret.Item".try_into().unwrap(),
            "Label",
            zbus::zvariant::Value::from("KBX Alpha renamed"),
        )
        .await
        .unwrap();
    let alpha_id = alpha.id.clone();
    expect_event(
        &mut events,
        "ItemChanged(alpha)",
        |e| matches!(e, ServiceEvent::ItemChanged { item, .. } if *item == alpha_id),
    )
    .await;
    let refreshed = client.item(&login.id, &alpha.id).await.unwrap();
    assert_eq!(refreshed.label.as_deref(), Some("KBX Alpha renamed"));

    // Another application deletes it.
    let reply: zbus::zvariant::OwnedObjectPath = other
        .call_method(
            Some("org.freedesktop.secrets"),
            alpha.id.path(),
            Some("org.freedesktop.Secret.Item"),
            "Delete",
            &(),
        )
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert_eq!(reply.as_str(), "/", "delete should not need a prompt");
    expect_event(
        &mut events,
        "ItemDeleted(alpha)",
        |e| matches!(e, ServiceEvent::ItemDeleted { item, .. } if *item == alpha_id),
    )
    .await;

    // The old entry now fails clearly instead of returning an empty secret.
    assert_eq!(
        client.item(&login.id, &alpha.id).await.unwrap_err(),
        Error::NoSuchObject
    );
    assert_eq!(
        client.secret(&alpha.id).await.unwrap_err(),
        Error::NoSuchObject
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn service_restart_invalidates_old_identities() {
    require_tools!();
    let mut env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let mut events = Box::pin(client.events().await.unwrap());
    let login = collection(&client, "Login").await;

    env.stop_oo7().await;
    expect_event(&mut events, "OwnerChanged", |e| {
        *e == ServiceEvent::OwnerChanged
    })
    .await;
    assert!(
        client.collections().await.is_err(),
        "old client must fail after owner loss"
    );

    env.start_oo7().await;
    // Calls are addressed to the old owner's unique name, so they cannot
    // reach the new daemon by accident.
    assert!(client.collections().await.is_err());

    let fresh = Client::connect(&env.bus()).await.unwrap();
    assert_ne!(fresh.generation(), client.generation());
    assert_eq!(fresh.items(&login.id).await.unwrap_err(), Error::Stale);
    assert_eq!(fresh.secret(&login.id).await.unwrap_err(), Error::Stale);
    assert!(collection(&fresh, "Login").await.error.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_missing_service() {
    require_tools!();
    let env = TestEnv::bus_only();
    assert_eq!(
        Client::connect(&env.bus()).await.unwrap_err(),
        Error::NoService
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_activation_failure() {
    require_tools!();
    let env = TestEnv::bus_only();
    env.install_failing_activation();
    assert_eq!(
        Client::connect(&env.bus()).await.unwrap_err(),
        Error::ActivationFailed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn debug_output_never_contains_fixture_data() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();

    let mut printed = format!("{client:?}");
    for c in client.collections().await.unwrap() {
        printed += &format!("{c:?}");
        if let Ok(items) = client.items(&c.id).await {
            for item in items {
                printed += &format!("{item:?}");
                printed += &format!("{:?}", client.secret(&item.id).await);
            }
        }
    }
    for needle in [
        "KBX-FIXTURE",
        "KBX Alpha",
        "KBX Duplicate",
        "alice",
        "example.invalid",
        "Markup",
    ] {
        assert!(!printed.contains(needle), "Debug output leaked {needle:?}");
    }
}
