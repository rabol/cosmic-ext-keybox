// SPDX-License-Identifier: GPL-3.0-only

//! Revealing and copying one selected secret.
//!
//! A secret is fetched only on an explicit Reveal or Copy, only for the
//! selected entry, and is dropped (wiped) as soon as it is hidden: on expiry,
//! selection change, navigation, window deactivation, service changes or
//! disconnect. Late results for anything but the current request are dropped.
//!
//! The clipboard is cleared after a delay only if it still holds exactly what
//! Keybox copied, checked by a salted hash. Keybox can only read the
//! clipboard reliably while its window is focused, so the check waits for
//! focus. Copies made by the toolkit, the compositor or clipboard managers
//! are outside Keybox's control.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use cosmic::prelude::*;
use zeroize::Zeroizing;

use keybox::backend::{Error, ItemInfo, ObjectId, RetrievedSecret, ServiceEvent};
use keybox::secret::{ClipboardFingerprint, SecretText, SecretValue};

use super::{App, Dialog, Message};
use crate::fl;

/// How long a copied secret may stay on the clipboard.
pub const CLIPBOARD_CLEAR_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Reveal,
    Copy,
}

pub enum SecretState {
    Hidden,
    Fetching {
        op: u64,
        item: ObjectId,
        intent: Intent,
    },
    Shown {
        item: ObjectId,
        value: SecretValue,
        content_type: String,
        expires_at: Instant,
        /// The entry's modification time when revealed; a change means the
        /// shown value may be stale.
        modified: Option<SystemTime>,
    },
    /// Copy was requested for text with control characters; waiting for the
    /// user to confirm in a dialog.
    AwaitingCopyConfirm {
        item: ObjectId,
        value: SecretValue,
    },
    Failed {
        item: ObjectId,
        error: Error,
    },
}

impl SecretState {
    pub fn item(&self) -> Option<&ObjectId> {
        match self {
            Self::Hidden => None,
            Self::Fetching { item, .. }
            | Self::Shown { item, .. }
            | Self::AwaitingCopyConfirm { item, .. }
            | Self::Failed { item, .. } => Some(item),
        }
    }

    /// Whether a secret value is held in memory.
    fn holds_value(&self) -> bool {
        matches!(self, Self::Shown { .. } | Self::AwaitingCopyConfirm { .. })
    }
}

/// Carries a retrieved secret through a message exactly once. Messages must
/// be `Clone + Debug`; this keeps the secret itself uncloneable and redacted.
#[derive(Clone)]
pub struct SecretSlot(Arc<Mutex<Option<Result<RetrievedSecret, Error>>>>);

impl std::fmt::Debug for SecretSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretSlot([REDACTED])")
    }
}

impl SecretSlot {
    fn new(result: Result<RetrievedSecret, Error>) -> Self {
        Self(Arc::new(Mutex::new(Some(result))))
    }

    fn take(&self) -> Option<Result<RetrievedSecret, Error>> {
        self.0.lock().ok()?.take()
    }
}

/// What to put on the clipboard. The toolkit needs an owned `String`; that
/// copy is outside Keybox's control once handed over.
struct ClipboardPayload {
    text: String,
    fingerprint: ClipboardFingerprint,
}

impl ClipboardPayload {
    fn new(text: &SecretText) -> Self {
        Self {
            text: text.expose().to_owned(),
            fingerprint: ClipboardFingerprint::new(text.expose()),
        }
    }
}

/// A pending best-effort clipboard clear.
pub struct ClipboardGuard {
    op: u64,
    fingerprint: Arc<ClipboardFingerprint>,
    clear_at: Instant,
    checking: bool,
}

impl App {
    /// Reveal or copy the selected entry's secret.
    pub(super) fn request_secret(&mut self, intent: Intent) -> Task<cosmic::Action<Message>> {
        let Some(item) = self.selected_item.clone() else {
            return Task::none();
        };

        // Copying a secret that is already shown needs no second retrieval.
        if intent == Intent::Copy
            && let SecretState::Shown {
                item: shown, value, ..
            } = &self.secret
            && shown == &item
        {
            let payload = value.copyable_text().map(ClipboardPayload::new);
            return match payload {
                Some(payload) => self.copy_payload(payload),
                None => self.toast(fl!("secret-binary-no-copy")),
            };
        }

        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.next_op += 1;
        let op = self.next_op;
        self.secret = SecretState::Fetching {
            op,
            item: item.clone(),
            intent,
        };
        cosmic::task::future(async move {
            let result = client.secret(&item).await;
            Message::SecretFetched(op, SecretSlot::new(result))
        })
    }

    pub(super) fn on_secret_fetched(
        &mut self,
        op: u64,
        slot: &SecretSlot,
    ) -> Task<cosmic::Action<Message>> {
        // Taking the result here means a stale one is dropped (and wiped) now.
        let result = slot.take();
        let SecretState::Fetching {
            op: current,
            item,
            intent,
        } = &self.secret
        else {
            return Task::none();
        };
        if *current != op || self.selected_item.as_ref() != Some(item) {
            return Task::none();
        }
        let (item, intent) = (item.clone(), *intent);
        let Some(result) = result else {
            return Task::none();
        };

        match result {
            Ok(retrieved) => {
                let RetrievedSecret {
                    value,
                    content_type,
                } = retrieved;
                match (intent, value) {
                    (Intent::Reveal, value) => {
                        let modified = self
                            .store
                            .as_ref()
                            .and_then(|s| s.item(&item))
                            .and_then(|i| i.modified);
                        self.secret = SecretState::Shown {
                            item,
                            value,
                            content_type,
                            expires_at: Instant::now() + self.reveal_duration(),
                            modified,
                        };
                        Task::none()
                    }
                    (Intent::Copy, SecretValue::Text(text)) => {
                        self.secret = SecretState::Hidden;
                        self.copy_text(&text)
                    }
                    (Intent::Copy, value @ SecretValue::TextWithControl(_)) => {
                        self.secret = SecretState::AwaitingCopyConfirm {
                            item: item.clone(),
                            value,
                        };
                        self.dialog = Some(Dialog::ConfirmCopy(item));
                        Task::none()
                    }
                    (Intent::Copy, SecretValue::Binary { .. }) => {
                        self.secret = SecretState::Hidden;
                        self.toast(fl!("secret-binary-no-copy"))
                    }
                }
            }
            Err(Error::Locked) => {
                self.secret = SecretState::Hidden;
                let collection = self
                    .store
                    .as_ref()
                    .and_then(|s| s.item(&item))
                    .map(|i| i.collection.clone());
                match collection {
                    Some(collection) if self.unlock.is_none() => {
                        self.after_unlock = Some((item, intent));
                        self.start_unlock(collection)
                    }
                    _ => {
                        self.secret = SecretState::Failed {
                            item,
                            error: Error::Locked,
                        };
                        Task::none()
                    }
                }
            }
            Err(error) => {
                self.secret = SecretState::Failed { item, error };
                Task::none()
            }
        }
    }

    /// Copies after the user confirmed text with control characters.
    pub(super) fn confirm_copy(&mut self, item: &ObjectId) -> Task<cosmic::Action<Message>> {
        self.dialog = None;
        let state = std::mem::replace(&mut self.secret, SecretState::Hidden);
        match state {
            SecretState::AwaitingCopyConfirm {
                item: pending,
                value: SecretValue::TextWithControl(text),
            } if &pending == item => self.copy_text(&text),
            _ => Task::none(),
        }
    }

    pub(super) fn cancel_copy(&mut self) {
        self.dialog = None;
        if matches!(self.secret, SecretState::AwaitingCopyConfirm { .. }) {
            self.secret = SecretState::Hidden;
        }
    }

    fn copy_text(&mut self, text: &SecretText) -> Task<cosmic::Action<Message>> {
        self.copy_payload(ClipboardPayload::new(text))
    }

    fn copy_payload(&mut self, payload: ClipboardPayload) -> Task<cosmic::Action<Message>> {
        self.next_op += 1;
        self.clipboard = Some(ClipboardGuard {
            op: self.next_op,
            fingerprint: Arc::new(payload.fingerprint),
            clear_at: Instant::now() + CLIPBOARD_CLEAR_AFTER,
            checking: false,
        });
        let write = cosmic::iced::clipboard::write(payload.text);
        let toast = self.toast(fl!(
            "secret-copied",
            seconds = CLIPBOARD_CLEAR_AFTER.as_secs()
        ));
        Task::batch([write, toast])
    }

    /// Hides any shown secret and drops it from memory.
    pub(super) fn hide_secret(&mut self) {
        if matches!(self.dialog, Some(Dialog::ConfirmCopy(_))) {
            self.dialog = None;
        }
        self.after_unlock = None;
        self.secret = SecretState::Hidden;
    }

    /// Hides the secret unless it belongs to `item`.
    pub(super) fn hide_secret_unless(&mut self, item: Option<&ObjectId>) {
        if self.secret.item() != item {
            self.hide_secret();
        }
    }

    /// Reacts to a service event concerning the shown secret. Deletion or
    /// loss of the service hides it at once. Other changes to the entry or its
    /// keyring trigger a fresh query of the entry, answered in
    /// [`App::on_reveal_check`]. Lists loaded earlier may be outdated, so
    /// they are not used for this decision.
    pub(super) fn check_secret_on_event(
        &mut self,
        event: &ServiceEvent,
    ) -> Task<cosmic::Action<Message>> {
        if matches!(event, ServiceEvent::OwnerChanged) {
            self.hide_secret();
            return Task::none();
        }
        // A pending fetch or confirmation is checked when it completes.
        let SecretState::Shown { item, .. } = &self.secret else {
            return Task::none();
        };
        let item = item.clone();
        let Some(collection) = self
            .store
            .as_ref()
            .and_then(|s| s.item(&item))
            .map(|i| i.collection.clone())
        else {
            self.hide_secret();
            return Task::none();
        };
        let concerns_us = match event {
            ServiceEvent::CollectionDeleted(c) if c == &collection => {
                self.hide_secret();
                return Task::none();
            }
            ServiceEvent::ItemDeleted { item: deleted, .. } if deleted == &item => {
                self.hide_secret();
                return Task::none();
            }
            ServiceEvent::CollectionChanged(c) => c == &collection,
            ServiceEvent::PropertiesChanged(object) => object == &collection || object == &item,
            ServiceEvent::ItemChanged { item: changed, .. } => changed == &item,
            _ => false,
        };
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        if !concerns_us {
            return Task::none();
        }
        cosmic::task::future(async move {
            let result = client.item(&collection, &item).await;
            Message::RevealCheck(item, result)
        })
    }

    /// Hides the shown secret if a fresh look at its entry shows it is gone,
    /// locked or modified since it was revealed.
    pub(super) fn on_reveal_check(&mut self, item: &ObjectId, result: Result<ItemInfo, Error>) {
        let SecretState::Shown {
            item: shown,
            modified,
            ..
        } = &mut self.secret
        else {
            return;
        };
        if shown != item {
            return;
        }
        let stale = match result {
            Err(_) => true,
            Ok(info) => info.locked != Some(false) || modified_changed(modified, info.modified),
        };
        if stale {
            self.hide_secret();
        }
    }

    pub(super) fn on_focus_changed(&mut self, focused: bool) -> Task<cosmic::Action<Message>> {
        self.window_focused = focused;
        if focused {
            // Run a clipboard check that was deferred while unfocused.
            self.tick()
        } else {
            if self.secret.holds_value()
                || matches!(
                    self.secret,
                    SecretState::Fetching {
                        intent: Intent::Reveal,
                        ..
                    }
                )
            {
                self.hide_secret();
            }
            Task::none()
        }
    }

    /// Once a second while something is time-limited.
    pub(super) fn tick(&mut self) -> Task<cosmic::Action<Message>> {
        let now = Instant::now();
        if let SecretState::Shown { expires_at, .. } = &self.secret
            && now >= *expires_at
        {
            self.hide_secret();
        }

        let Some(guard) = self.clipboard.as_mut() else {
            return Task::none();
        };
        if now < guard.clear_at || guard.checking || !self.window_focused {
            return Task::none();
        }
        guard.checking = true;
        let op = guard.op;
        let fingerprint = guard.fingerprint.clone();
        cosmic::iced::clipboard::read().map(move |content| {
            // Hash and wipe immediately; the content never enters a message.
            let content = content.map(Zeroizing::new);
            let ours = content.as_deref().map(|text| fingerprint.matches(text));
            cosmic::Action::App(Message::ClipboardChecked(op, ours))
        })
    }

    pub(super) fn on_clipboard_checked(
        &mut self,
        op: u64,
        ours: Option<bool>,
    ) -> Task<cosmic::Action<Message>> {
        if self.clipboard.as_ref().is_none_or(|g| g.op != op) {
            return Task::none();
        }
        self.clipboard = None;
        if ours == Some(true) {
            let clear = cosmic::iced::clipboard::write(String::new());
            let toast = self.toast(fl!("secret-clipboard-cleared"));
            Task::batch([clear, toast])
        } else {
            // Replaced by something newer, or unreadable: leave it alone.
            Task::none()
        }
    }

    pub(super) fn needs_tick(&self) -> bool {
        matches!(self.secret, SecretState::Shown { .. }) || self.clipboard.is_some()
    }

    fn reveal_duration(&self) -> Duration {
        Duration::from_secs(u64::from(self.config.reveal_seconds.clamp(5, 600)))
    }
}

/// Whether the modification time changed. An unknown earlier time adopts
/// the first known one instead of counting as a change.
fn modified_changed(recorded: &mut Option<SystemTime>, now: Option<SystemTime>) -> bool {
    match (*recorded, now) {
        (Some(before), Some(now)) => before != now,
        (None, now) => {
            *recorded = now;
            false
        }
        (Some(_), None) => false,
    }
}
