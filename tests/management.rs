// SPDX-License-Identifier: GPL-3.0-only

//! Integration tests for changes to stored data (the management release),
//! run against isolated oo7 and GNOME Keyring daemons.
//!
//! Kept separate from the read-only tests on purpose: everything here
//! writes, and only ever to the disposable keyrings in `support/mod.rs`.

#![allow(clippy::pedantic)]

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use keybox::backend::{Client, CollectionInfo, Error, ObjectId, PromptOutcome, ServiceEvent};
use keybox::secret::{SecretText, SecretValue};
use support::{PromptBehaviour, TestEnv};
use zeroize::Zeroizing;

const NEW_SECRET: &str = "KBX-FIXTURE-NEW-SECRET";
const CHANGED_SECRET: &str = "KBX-FIXTURE-CHANGED-SECRET";

fn text(s: &str) -> SecretText {
    SecretText::new(Zeroizing::new(s.to_owned()))
}

fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

async fn default_collection(client: &Client) -> CollectionInfo {
    client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.is_default == Some(true))
        .expect("default collection")
}

async fn secret_text(client: &Client, item: &ObjectId) -> String {
    match client.secret(item).await.unwrap().value {
        SecretValue::Text(t) => t.expose().to_owned(),
        other => panic!("expected text, got {other:?}"),
    }
}

async fn expect_event(
    events: &mut (impl Stream<Item = ServiceEvent> + Unpin),
    what: &str,
    pred: impl Fn(&ServiceEvent) -> bool,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(event)) if pred(&event) => return,
            Ok(Some(_)) => {}
            Ok(None) => panic!("event stream ended while waiting for {what}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

// --- scenarios shared by both providers ---

/// Create, then create again with the same attributes: two separate
/// entries, never an implicit replacement. Secrets travel encrypted.
async fn create_never_replaces(env: &TestEnv) {
    let recorder = support::write_call_recorder(env).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = default_collection(&client).await;
    let mut events = Box::pin(client.events().await.unwrap());
    let a = attrs(&[("kbx-new", "1"), ("user", "bob")]);

    let first = client
        .create_item(&login.id, "KBX New", &a, &text(NEW_SECRET), None)
        .await
        .unwrap();
    expect_event(
        &mut events,
        "ItemCreated",
        |e| matches!(e, ServiceEvent::ItemCreated { item, .. } if *item == first),
    )
    .await;
    let info = client.item(&login.id, &first).await.unwrap();
    assert_eq!(info.label.as_deref(), Some("KBX New"));
    let stored = info.attributes.unwrap();
    for (k, v) in &a {
        assert_eq!(stored.get(k), Some(v), "attribute {k}");
    }
    assert_eq!(secret_text(&client, &first).await, NEW_SECRET);

    assert_eq!(
        client.find_items(&login.id, &a).await.unwrap(),
        vec![first.clone()]
    );
    let second = client
        .create_item(
            &login.id,
            "KBX New again",
            &a,
            &text("KBX-FIXTURE-OTHER"),
            None,
        )
        .await
        .unwrap();
    assert_ne!(first, second);
    let mut found = client.find_items(&login.id, &a).await.unwrap();
    found.sort_by(|x, y| x.path().cmp(y.path()));
    let mut expected = vec![first.clone(), second];
    expected.sort_by(|x, y| x.path().cmp(y.path()));
    assert_eq!(found, expected);
    assert_eq!(
        secret_text(&client, &first).await,
        NEW_SECRET,
        "first entry untouched"
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    let calls = recorder.lock().unwrap();
    assert_eq!(calls.len(), 2, "two CreateItem calls observed");
    for call in calls.iter() {
        assert!(!support::contains_bytes(call, NEW_SECRET.as_bytes()));
        assert!(!support::contains_bytes(call, b"KBX-FIXTURE-OTHER"));
    }
}

/// Label, attributes and secret can each be changed; other entries stay.
async fn edit_entry(env: &TestEnv) {
    let recorder = support::write_call_recorder(env).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = default_collection(&client).await;
    let item = client
        .create_item(
            &login.id,
            "KBX Edit",
            &attrs(&[("kbx-edit", "1")]),
            &text(NEW_SECRET),
            None,
        )
        .await
        .unwrap();
    let mut events = Box::pin(client.events().await.unwrap());

    client.set_item_label(&item, "KBX Edited").await.unwrap();
    expect_event(
        &mut events,
        "ItemChanged after label",
        |e| matches!(e, ServiceEvent::ItemChanged { item: i, .. } if *i == item),
    )
    .await;
    let new_attrs = attrs(&[("kbx-edit", "2"), ("user", "carol")]);
    client.set_item_attributes(&item, &new_attrs).await.unwrap();
    client
        .set_item_secret(&item, &text(CHANGED_SECRET))
        .await
        .unwrap();

    let info = client.item(&login.id, &item).await.unwrap();
    assert_eq!(info.label.as_deref(), Some("KBX Edited"));
    let stored = info.attributes.unwrap();
    for (k, v) in &new_attrs {
        assert_eq!(stored.get(k), Some(v), "attribute {k}");
    }
    assert!(!stored.contains_key("kbx-test"));
    assert_eq!(stored.get("kbx-edit").map(String::as_str), Some("2"));
    assert_eq!(secret_text(&client, &item).await, CHANGED_SECRET);

    let alpha = client
        .find_items(&login.id, &attrs(&[("kind", "alpha")]))
        .await
        .unwrap();
    assert_eq!(alpha.len(), 1);
    assert_eq!(secret_text(&client, &alpha[0]).await, support::SECRET_ALPHA);

    tokio::time::sleep(Duration::from_millis(200)).await;
    for call in recorder.lock().unwrap().iter() {
        assert!(!support::contains_bytes(call, CHANGED_SECRET.as_bytes()));
        assert!(!support::contains_bytes(call, NEW_SECRET.as_bytes()));
    }
}

/// Deleting an entry removes exactly that entry and reports it.
async fn delete_entry(env: &TestEnv) {
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = default_collection(&client).await;
    let dups = client
        .find_items(&login.id, &attrs(&[("kind", "dup")]))
        .await
        .unwrap();
    assert_eq!(dups.len(), 2);
    let mut events = Box::pin(client.events().await.unwrap());

    assert_eq!(
        client.delete_item(&dups[0], None).await.unwrap(),
        PromptOutcome::Done
    );
    expect_event(
        &mut events,
        "ItemDeleted",
        |e| matches!(e, ServiceEvent::ItemDeleted { item, .. } if *item == dups[0]),
    )
    .await;
    assert_eq!(
        client.item(&login.id, &dups[0]).await.unwrap_err(),
        Error::NoSuchObject
    );
    // The other entry with the same label is untouched.
    assert!(client.item(&login.id, &dups[1]).await.is_ok());
    let left = client
        .find_items(&login.id, &attrs(&[("kind", "dup")]))
        .await
        .unwrap();
    assert_eq!(left, vec![dups[1].clone()]);
}

/// Rename, default alias, and refusal to delete a non-empty keyring.
async fn keyring_metadata(env: &TestEnv) {
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = default_collection(&client).await;
    let original = login.label.clone().unwrap();
    let mut events = Box::pin(client.events().await.unwrap());

    client
        .set_collection_label(&login.id, "KBX Renamed keyring")
        .await
        .unwrap();
    expect_event(&mut events, "change after rename", |e| {
        matches!(e, ServiceEvent::CollectionChanged(c) | ServiceEvent::PropertiesChanged(c) if *c == login.id)
    })
    .await;
    let renamed = default_collection(&client).await;
    assert_eq!(renamed.label.as_deref(), Some("KBX Renamed keyring"));
    client
        .set_collection_label(&login.id, &original)
        .await
        .unwrap();

    assert_eq!(
        client
            .delete_empty_collection(&login.id, None)
            .await
            .unwrap_err(),
        Error::NotEmpty
    );
    assert!(
        !client.items(&login.id).await.unwrap().is_empty(),
        "nothing deleted"
    );

    let session = client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.is_session)
        .expect("a session keyring is marked");
    assert_ne!(session.id, login.id);
    assert!(!login.is_session);
}

// --- oo7 ---

macro_rules! require_tools {
    () => {
        if !support::tools_available() {
            return;
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn oo7_create_never_replaces() {
    require_tools!();
    create_never_replaces(&TestEnv::oo7().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn oo7_edit_entry() {
    require_tools!();
    edit_entry(&TestEnv::oo7().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn oo7_delete_entry() {
    require_tools!();
    delete_entry(&TestEnv::oo7().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn oo7_keyring_metadata() {
    require_tools!();
    keyring_metadata(&TestEnv::oo7().await).await;
}

/// oo7 asks its prompter for a new keyring's password. With a prompter
/// that answers, the keyring is created; it can become the default, be
/// renamed and, while empty, be deleted.
#[tokio::test(flavor = "multi_thread")]
async fn oo7_keyring_lifecycle_with_prompter() {
    require_tools!();
    let (env, prompter) =
        TestEnv::oo7_with_prompter(PromptBehaviour::Accept(vec!["kbx-new-password".into()])).await;
    let client = Client::connect(&env.bus()).await.unwrap();

    let extra = client
        .create_collection("KBX Extra", Some("wayland:kbx-test"))
        .await
        .unwrap();
    assert_eq!(prompter.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        prompter.window_ids.lock().unwrap().as_slice(),
        ["wayland:kbx-test"]
    );
    let info = client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.id == extra)
        .unwrap();
    assert_eq!(info.label.as_deref(), Some("KBX Extra"));
    assert_eq!(info.locked, Some(false));
    assert_eq!(info.is_default, Some(false));
    assert!(!info.is_session);

    client
        .set_collection_label(&extra, "KBX Extra renamed")
        .await
        .unwrap();
    assert_eq!(
        client.delete_empty_collection(&extra, None).await.unwrap(),
        PromptOutcome::Done
    );
    assert!(
        client
            .collections()
            .await
            .unwrap()
            .iter()
            .all(|c| c.id != extra)
    );

    // oo7 0.6.0 marks the target as "default" without unmarking the old
    // keyring, and ReadAlias returns whichever it finds first. Keybox must
    // never report success unless the alias really points at the target.
    let other = client.create_collection("KBX Other", None).await.unwrap();
    match client.set_default_collection(&other).await {
        Ok(()) => assert_eq!(default_collection(&client).await.id, other),
        Err(error) => {
            assert_eq!(error, Error::NotApplied);
            assert_ne!(default_collection(&client).await.id, other);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn oo7_keyring_creation_cancelled() {
    require_tools!();
    let (env, _prompter) = TestEnv::oo7_with_prompter(PromptBehaviour::Reject).await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let before = client.collections().await.unwrap().len();
    assert_eq!(
        client
            .create_collection("KBX Extra", None)
            .await
            .unwrap_err(),
        Error::PromptDismissed
    );
    assert_eq!(client.collections().await.unwrap().len(), before);
}

/// Without a prompter (as on COSMIC today), oo7 never answers a keyring
/// creation or a change to a locked keyring. Dropping the request must
/// leave the client usable.
#[tokio::test(flavor = "multi_thread")]
async fn oo7_unanswered_write_prompts_can_be_abandoned() {
    require_tools!();
    let env = TestEnv::oo7().await;
    let client = Client::connect(&env.bus()).await.unwrap();
    let work = client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.label.as_deref() == Some("Work"))
        .unwrap();
    assert_eq!(work.locked, Some(true));

    let create = tokio::time::timeout(
        Duration::from_secs(2),
        client.create_collection("KBX Extra", None),
    )
    .await;
    assert!(create.is_err(), "keyring creation should wait for a prompt");
    let write = tokio::time::timeout(
        Duration::from_secs(2),
        client.create_item(
            &work.id,
            "KBX Locked",
            &attrs(&[("k", "v")]),
            &text(NEW_SECRET),
            None,
        ),
    )
    .await;
    assert!(
        write.is_err(),
        "writing to a locked keyring should wait for a prompt"
    );

    // Locked entries refuse a new secret outright.
    let item = client.items(&work.id).await.unwrap().remove(0);
    assert_eq!(
        client
            .set_item_secret(&item.id, &text(NEW_SECRET))
            .await
            .unwrap_err(),
        Error::Locked
    );
    assert!(client.collections().await.is_ok());
}

// --- GNOME Keyring ---

macro_rules! gnome_or_skip {
    () => {
        match TestEnv::gnome_keyring().await {
            Some(env) => env,
            None => return,
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn gnome_keyring_create_never_replaces() {
    create_never_replaces(&gnome_or_skip!()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gnome_keyring_edit_entry() {
    edit_entry(&gnome_or_skip!()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gnome_keyring_delete_entry() {
    delete_entry(&gnome_or_skip!()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gnome_keyring_keyring_metadata() {
    keyring_metadata(&gnome_or_skip!()).await;
}

/// The default alias can move away from the login keyring and back.
#[tokio::test(flavor = "multi_thread")]
async fn gnome_keyring_default_alias_moves_both_ways() {
    let env = gnome_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let login = default_collection(&client).await;
    let session = client
        .collections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.is_session)
        .unwrap();
    client.set_default_collection(&session.id).await.unwrap();
    assert_eq!(default_collection(&client).await.id, session.id);
    client.set_default_collection(&login.id).await.unwrap();
    assert_eq!(default_collection(&client).await.id, login.id);
}

/// GNOME Keyring refuses changes to a locked keyring with `IsLocked`, and
/// without a prompter dismisses keyring creation immediately.
#[tokio::test(flavor = "multi_thread")]
async fn gnome_keyring_locked_and_creation_without_prompter() {
    let env = gnome_or_skip!();
    let client = Client::connect(&env.bus()).await.unwrap();
    let before = client.collections().await.unwrap().len();
    assert_eq!(
        client
            .create_collection("KBX Extra", None)
            .await
            .unwrap_err(),
        Error::PromptDismissed
    );
    assert_eq!(client.collections().await.unwrap().len(), before);

    let login = default_collection(&client).await;
    let item = client.items(&login.id).await.unwrap().remove(0);
    client
        .lock(std::slice::from_ref(&login.id), None)
        .await
        .unwrap();
    assert_eq!(
        client
            .create_item(
                &login.id,
                "KBX Locked",
                &attrs(&[("k", "v")]),
                &text(NEW_SECRET),
                None
            )
            .await
            .unwrap_err(),
        Error::Locked
    );
    assert_eq!(
        client.set_item_label(&item.id, "x").await.unwrap_err(),
        Error::Locked
    );
    assert_eq!(
        client
            .set_item_secret(&item.id, &text(NEW_SECRET))
            .await
            .unwrap_err(),
        Error::Locked
    );
    assert_eq!(
        client.delete_item(&item.id, None).await.unwrap_err(),
        Error::Locked
    );
}
