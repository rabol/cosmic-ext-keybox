// SPDX-License-Identifier: GPL-3.0-only

//! Development hooks, compiled into debug builds only.
//!
//! - `KEYBOX_DEV_BUS=<address>`: use this bus instead of the session bus,
//!   for example the isolated bus from `cargo run --example dev_env`.
//! - `KEYBOX_DEV_SCRIPT=<steps>`: run `;`-separated steps after start-up and
//!   save screenshots of Keybox's own window (never the whole screen):
//!   `wait:<ms>`, `keyring:<label>`, `select:<label>`, `search:<text>`,
//!   `search-close`, `size:<w>x<h>`, `unlock`, `lock`, `confirm`, `escape`,
//!   `back`, `technical`, `single-pane`, `reveal`, `copy`, `hide`, `unfocus`,
//!   `focus`, `clip-is:<text>`, `clip-set:<text>`, `settings`,
//!   `reveal-choice:<index>`, `key:<keys>`, `shot:<file.png>`, `quit`.
//! - Changing data: `new-entry`, `edit-entry`, `delete-entry`, `new-keyring`,
//!   `rename-keyring`, `delete-keyring`, `add-attr`, `remove-attr:<i>`,
//!   `replace-secret`, `show-secret`, `save`, `editor-confirm`,
//!   `editor-reload`, `editor-cancel`, `cancel-write`, `status` (prints
//!   editor, dialog and write state without any text), and field input:
//!   `label=<text>`, `secret=<text>`, `attr-key<i>=<text>`,
//!   `attr-value<i>=<text>`, `dialog=<text>`.
//! - Built-in password prompt: `password=<text>`, `prompt-submit`,
//!   `prompt-cancel`.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Step {
    Wait(Duration),
    Keyring(String),
    Select(String),
    Search(String),
    SearchClose,
    Size(f32, f32),
    Unlock,
    Lock,
    Confirm,
    Escape,
    Back,
    Technical,
    /// Force the one-pane layout regardless of window width.
    SinglePane,
    Reveal,
    Copy,
    Hide,
    /// Simulate the window losing or gaining focus.
    Focus(bool),
    /// Print whether the clipboard currently equals this text. Only for
    /// synthetic fixtures; never prints clipboard contents.
    ClipIs(String),
    /// Put other text on the clipboard, as another app would.
    ClipSet(String),
    Settings,
    /// Press a key as if no widget handled it, e.g. `down`, `ctrl+pagedown`,
    /// `ctrl+shift+e`.
    Key(String),
    /// Choose an entry in the reveal-duration dropdown by index.
    RevealChoice(usize),
    Shot(PathBuf),
    Status,
    Quit,
    /// Send an app message directly (buttons without their own step).
    Message(Box<super::Message>),
    /// Type into an editor or dialog field.
    Field(Field, String),
}

#[derive(Debug, Clone, Copy)]
pub enum Field {
    Label,
    Secret,
    AttributeKey(usize),
    AttributeValue(usize),
    Dialog,
    #[cfg(feature = "builtin-prompter")]
    Password,
}

#[must_use]
pub fn bus_override() -> Option<String> {
    std::env::var("KEYBOX_DEV_BUS")
        .ok()
        .filter(|s| !s.is_empty())
}

#[must_use]
pub fn script() -> Vec<Step> {
    let Ok(script) = std::env::var("KEYBOX_DEV_SCRIPT") else {
        return Vec::new();
    };
    script
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|step| {
            use super::Message as M;
            if let Some((field, text)) = step.split_once('=') {
                let field = match field {
                    "label" => Field::Label,
                    "secret" => Field::Secret,
                    "dialog" => Field::Dialog,
                    #[cfg(feature = "builtin-prompter")]
                    "password" => Field::Password,
                    f => {
                        if let Some(i) = f.strip_prefix("attr-key") {
                            Field::AttributeKey(i.parse().ok()?)
                        } else if let Some(i) = f.strip_prefix("attr-value") {
                            Field::AttributeValue(i.parse().ok()?)
                        } else {
                            eprintln!("KEYBOX_DEV_SCRIPT: unknown field {f:?}");
                            return None;
                        }
                    }
                };
                return Some(Step::Field(field, text.to_owned()));
            }
            let (name, arg) = step.split_once(':').unwrap_or((step, ""));
            Some(match name {
                "wait" => Step::Wait(Duration::from_millis(arg.parse().ok()?)),
                "keyring" => Step::Keyring(arg.to_owned()),
                "select" => Step::Select(arg.to_owned()),
                "search" => Step::Search(arg.to_owned()),
                "search-close" => Step::SearchClose,
                "size" => {
                    let (w, h) = arg.split_once('x')?;
                    Step::Size(w.parse().ok()?, h.parse().ok()?)
                }
                "unlock" => Step::Unlock,
                "lock" => Step::Lock,
                "confirm" => Step::Confirm,
                "escape" => Step::Escape,
                "back" => Step::Back,
                "technical" => Step::Technical,
                "single-pane" => Step::SinglePane,
                "reveal" => Step::Reveal,
                "copy" => Step::Copy,
                "hide" => Step::Hide,
                "unfocus" => Step::Focus(false),
                "focus" => Step::Focus(true),
                "settings" => Step::Settings,
                "key" => Step::Key(arg.to_owned()),
                "reveal-choice" => Step::RevealChoice(arg.parse().ok()?),
                "clip-set" => Step::ClipSet(arg.to_owned()),
                "clip-is" => Step::ClipIs(arg.replace("\\n", "\n").replace("\\0", "\0")),
                "shot" => Step::Shot(PathBuf::from(arg)),
                "quit" => Step::Quit,
                "status" => Step::Status,
                "new-entry" => Step::Message(Box::new(M::NewEntry)),
                "edit-entry" => Step::Message(Box::new(M::EditEntry)),
                "delete-entry" => Step::Message(Box::new(M::RequestDeleteEntry)),
                "new-keyring" => Step::Message(Box::new(M::RequestNewKeyring)),
                "rename-keyring" => Step::Message(Box::new(M::RequestRenameKeyring)),
                "delete-keyring" => Step::Message(Box::new(M::RequestDeleteKeyring)),
                "add-attr" => Step::Message(Box::new(M::EditorAddAttribute)),
                "remove-attr" => {
                    Step::Message(Box::new(M::EditorRemoveAttribute(arg.parse().ok()?)))
                }
                "replace-secret" => Step::Message(Box::new(M::EditorReplaceSecret(true))),
                "show-secret" => Step::Message(Box::new(M::EditorSecretVisible)),
                "save" => Step::Message(Box::new(M::EditorSave)),
                "editor-confirm" => Step::Message(Box::new(M::EditorConfirm)),
                "editor-reload" => Step::Message(Box::new(M::EditorReload)),
                "editor-cancel" => Step::Message(Box::new(M::EditorCancel)),
                "cancel-write" => Step::Message(Box::new(M::CancelWrite)),
                #[cfg(feature = "builtin-prompter")]
                "prompt-submit" => Step::Message(Box::new(M::PasswordPrompt(
                    super::prompter::PromptMessage::Submit,
                ))),
                #[cfg(feature = "builtin-prompter")]
                "prompt-cancel" => Step::Message(Box::new(M::PasswordPrompt(
                    super::prompter::PromptMessage::Cancel,
                ))),
                other => {
                    eprintln!("KEYBOX_DEV_SCRIPT: unknown step {other:?}");
                    return None;
                }
            })
        })
        .collect()
}

pub fn save_png(path: &std::path::Path, shot: &cosmic::iced::window::Screenshot) {
    let result = image::save_buffer(
        path,
        &shot.rgba,
        shot.size.width,
        shot.size.height,
        image::ExtendedColorType::Rgba8,
    );
    match result {
        Ok(()) => eprintln!("KEYBOX_DEV_SCRIPT: saved {}", path.display()),
        Err(err) => eprintln!(
            "KEYBOX_DEV_SCRIPT: could not save {}: {err}",
            path.display()
        ),
    }
}

/// Parses `ctrl+shift+e`, `down`, `ctrl+pagedown` into a key press.
#[must_use]
pub fn parse_key(
    spec: &str,
) -> Option<(
    cosmic::iced::keyboard::Modifiers,
    cosmic::iced::keyboard::Key,
)> {
    use cosmic::iced::keyboard::{Key, Modifiers, key::Named};
    let mut modifiers = Modifiers::empty();
    let mut key = None;
    for part in spec.split('+') {
        match part {
            "ctrl" => modifiers |= Modifiers::CTRL,
            "shift" => modifiers |= Modifiers::SHIFT,
            "alt" => modifiers |= Modifiers::ALT,
            "down" => key = Some(Key::Named(Named::ArrowDown)),
            "up" => key = Some(Key::Named(Named::ArrowUp)),
            "home" => key = Some(Key::Named(Named::Home)),
            "end" => key = Some(Key::Named(Named::End)),
            "pagedown" => key = Some(Key::Named(Named::PageDown)),
            "pageup" => key = Some(Key::Named(Named::PageUp)),
            c if c.chars().count() == 1 => key = Some(Key::Character(c.into())),
            _ => return None,
        }
    }
    Some((modifiers, key?))
}
