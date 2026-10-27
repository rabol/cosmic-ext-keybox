// SPDX-License-Identifier: GPL-3.0-only

//! Keybox's own password dialog for the built-in prompter
//! (`keybox::backend::prompter`). Temporary: remove with the
//! `builtin-prompter` feature once COSMIC ships a keyring prompter.

use std::future::Future;
use std::sync::{Arc, Mutex};

use cosmic::prelude::*;
use cosmic::widget;
use futures_util::{Stream, StreamExt, stream};
use keybox::backend::Client;
use keybox::backend::prompter::{PasswordRequest, PromptKind, Prompter, Requests};
use zeroize::Zeroizing;

use super::editor::TypedSecret;
use super::{App, Message};
use crate::fl;

/// A provider's password request, shareable so it fits in a message.
#[derive(Clone)]
pub struct SharedRequest(Arc<Mutex<Option<PasswordRequest>>>);

impl std::fmt::Debug for SharedRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedRequest")
    }
}

impl SharedRequest {
    fn take(&self) -> Option<PasswordRequest> {
        self.0.lock().ok()?.take()
    }
}

#[derive(Debug, Clone)]
pub enum PromptMessage {
    /// The provider asks for a password for operation `op` on `keyring`.
    Request(u64, String, SharedRequest),
    Password(TypedSecret),
    Confirm(TypedSecret),
    ToggleVisible,
    Submit,
    Cancel,
}

/// The open password dialog. Typed text lives in wiping buffers.
pub struct PasswordDialog {
    op: u64,
    keyring: String,
    kind: PromptKind,
    retry: bool,
    password: Zeroizing<String>,
    confirm: Zeroizing<String>,
    hidden: bool,
    request: SharedRequest,
}

impl PasswordDialog {
    fn can_submit(&self) -> bool {
        match self.kind {
            PromptKind::Unlock => !self.password.is_empty(),
            PromptKind::NewPassword => *self.password == *self.confirm,
        }
    }
}

impl Drop for PasswordDialog {
    fn drop(&mut self) {
        // Closing the dialog without an answer cancels the prompt.
        if let Some(request) = self.request.take() {
            request.cancel();
        }
    }
}

/// Runs `work` (which receives the window identifier to pass to the
/// provider) with the built-in prompter active, when the desktop has no
/// prompter of its own. Password requests arrive as
/// [`PromptMessage::Request`]; the stream ends with `work`'s message.
pub fn with_prompter<F>(
    client: Client,
    op: u64,
    keyring: String,
    work: impl FnOnce(Option<String>) -> F + Send + 'static,
) -> impl Stream<Item = Message> + Send + 'static
where
    F: Future<Output = Message> + Send + 'static,
{
    stream::once(async move {
        let started = Prompter::start(&client).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "built-in prompter unavailable");
            None
        });
        let (prompter, requests) = started.unzip();
        let work = Box::pin(work(prompter.as_ref().map(|p| p.window_id().to_owned())));
        stream::unfold(Some((work, requests, prompter)), move |state| {
            let keyring = keyring.clone();
            async move {
                let (mut work, mut requests, prompter) = state?;
                tokio::select! {
                    message = &mut work => {
                        // Release the prompter name before reporting.
                        drop(prompter);
                        Some((message, None))
                    }
                    Some(request) = next_request(&mut requests) => {
                        let shared = SharedRequest(Arc::new(Mutex::new(Some(request))));
                        let message =
                            Message::PasswordPrompt(PromptMessage::Request(op, keyring, shared));
                        Some((message, Some((work, requests, prompter))))
                    }
                }
            }
        })
    })
    .flatten()
}

async fn next_request(requests: &mut Option<Requests>) -> Option<PasswordRequest> {
    match requests {
        Some(requests) => requests.next().await,
        None => std::future::pending().await,
    }
}

impl App {
    /// Whether `op` is still a pending unlock or change.
    fn prompt_op_active(&self, op: u64) -> bool {
        self.unlock.as_ref().is_some_and(|u| u.op == op)
            || self.write.as_ref().is_some_and(|w| w.op == op)
    }

    pub(super) fn on_password_prompt(
        &mut self,
        message: PromptMessage,
    ) -> Task<cosmic::Action<Message>> {
        match message {
            PromptMessage::Request(op, keyring, request) => {
                if !self.prompt_op_active(op) {
                    if let Some(request) = request.take() {
                        request.cancel();
                    }
                    return Task::none();
                }
                let Some(pending) = request
                    .0
                    .lock()
                    .ok()
                    .and_then(|r| r.as_ref().map(|r| (r.kind, r.retry)))
                else {
                    return Task::none();
                };
                self.password_prompt = Some(PasswordDialog {
                    op,
                    keyring,
                    kind: pending.0,
                    retry: pending.1,
                    password: Zeroizing::new(String::new()),
                    confirm: Zeroizing::new(String::new()),
                    hidden: true,
                    request,
                });
                return widget::text_input::focus(self.dialog_input_id.clone());
            }
            PromptMessage::Password(text) => {
                if let Some(dialog) = self.password_prompt.as_mut() {
                    dialog.password = Zeroizing::new(text.take());
                }
            }
            PromptMessage::Confirm(text) => {
                if let Some(dialog) = self.password_prompt.as_mut() {
                    dialog.confirm = Zeroizing::new(text.take());
                }
            }
            PromptMessage::ToggleVisible => {
                if let Some(dialog) = self.password_prompt.as_mut() {
                    dialog.hidden = !dialog.hidden;
                }
            }
            PromptMessage::Submit => {
                if let Some(mut dialog) = self.password_prompt.take_if(|d| d.can_submit())
                    && let Some(request) = dialog.request.take()
                {
                    // The banner shows progress; a wrong password reopens
                    // the dialog with a warning.
                    request.submit(std::mem::take(&mut dialog.password));
                }
            }
            // Dropping the dialog cancels the prompt.
            PromptMessage::Cancel => self.password_prompt = None,
        }
        Task::none()
    }

    /// The password dialog, while one of Keybox's requests needs it.
    pub(super) fn password_dialog(&self) -> Option<Element<'_, Message>> {
        let dialog = self.password_prompt.as_ref()?;
        if !self.prompt_op_active(dialog.op) {
            return None;
        }
        let msg = Message::PasswordPrompt;
        let submit = dialog.can_submit().then_some(msg(PromptMessage::Submit));
        let (title, body, confirm_label) = match dialog.kind {
            PromptKind::Unlock => (
                fl!(
                    "password-prompt-unlock-title",
                    keyring = dialog.keyring.clone()
                ),
                fl!("password-prompt-unlock-body"),
                fl!("password-prompt-unlock"),
            ),
            PromptKind::NewPassword => (
                fl!(
                    "password-prompt-new-title",
                    keyring = dialog.keyring.clone()
                ),
                fl!("password-prompt-new-body"),
                fl!("password-prompt-create"),
            ),
        };

        let mut controls = widget::column::with_capacity(4).spacing(12);
        if dialog.retry {
            controls = controls.push(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .align_y(cosmic::iced::Alignment::Center)
                    .push(widget::icon::from_name("dialog-warning-symbolic").size(16))
                    .push(widget::text::body(fl!("password-prompt-wrong"))),
            );
        }
        let mut password = widget::secure_input(
            fl!("password-prompt-password"),
            dialog.password.as_str(),
            Some(msg(PromptMessage::ToggleVisible)),
            dialog.hidden,
        )
        .label(fl!("password-prompt-password"))
        .id(self.dialog_input_id.clone())
        .on_input(|text| Message::PasswordPrompt(PromptMessage::Password(TypedSecret::new(text))));
        if dialog.kind == PromptKind::Unlock {
            password = password.on_submit_maybe(submit.clone().map(|m| move |_| m.clone()));
        }
        controls = controls.push(password);
        if dialog.kind == PromptKind::NewPassword {
            controls = controls.push(
                widget::secure_input(
                    fl!("password-prompt-confirm"),
                    dialog.confirm.as_str(),
                    None,
                    dialog.hidden,
                )
                .label(fl!("password-prompt-confirm"))
                .on_input(|text| {
                    Message::PasswordPrompt(PromptMessage::Confirm(TypedSecret::new(text)))
                })
                .on_submit_maybe(submit.clone().map(|m| move |_| m.clone())),
            );
            if !dialog.confirm.is_empty() && *dialog.password != *dialog.confirm {
                controls = controls.push(widget::text::caption(fl!("password-prompt-mismatch")));
            }
        }
        controls = controls.push(widget::text::caption(fl!("password-prompt-note")));

        Some(
            widget::dialog()
                .title(title)
                .body(body)
                .icon(widget::icon::from_name("dialog-password-symbolic").size(64))
                .control(controls)
                .primary_action(widget::button::suggested(confirm_label).on_press_maybe(submit))
                .secondary_action(
                    widget::button::standard(fl!("cancel")).on_press(msg(PromptMessage::Cancel)),
                )
                .into(),
        )
    }
}
