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
    Quit,
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
