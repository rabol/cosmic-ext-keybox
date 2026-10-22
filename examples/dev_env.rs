// SPDX-License-Identifier: GPL-3.0-only

//! Starts the isolated test environment (private bus + oo7-daemon with
//! synthetic keyrings and a fake prompter) for manual testing, then waits.
//!
//! ```sh
//! cargo run --example dev_env            # prints KEYBOX_DEV_BUS=...
//! KEYBOX_DEV_BUS=... cargo run           # in another terminal
//! ```
//!
//! The fake prompter accepts the Work keyring password. Arguments: `ignore`
//! or `reject` change the prompter; `gnome` runs GNOME Keyring instead of
//! oo7 (see `KEYBOX_TEST_GNOME_KEYRING_ROOT` in `tests/support`).
//! Press Enter to stop.

#[path = "../tests/support/mod.rs"]
mod support;

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("gnome") {
        let Some(env) = support::TestEnv::gnome_keyring().await else {
            eprintln!("No gnome-keyring-daemon; set KEYBOX_TEST_GNOME_KEYRING_ROOT.");
            return;
        };
        println!("KEYBOX_DEV_BUS={}", env.address());
        eprintln!("Isolated GNOME Keyring running. Press Enter to stop.");
        wait_for_enter().await;
        return;
    }
    let behaviour = match std::env::args().nth(1).as_deref() {
        Some("ignore") => support::PromptBehaviour::Ignore,
        Some("reject") => support::PromptBehaviour::Reject,
        _ => support::PromptBehaviour::Accept(vec![
            support::WORK_PASSWORD.into(),
            support::LOGIN_PASSWORD.into(),
        ]),
    };
    let (env, _prompter) = support::TestEnv::oo7_with_prompter(behaviour).await;
    println!("KEYBOX_DEV_BUS={}", env.address());
    eprintln!("Isolated keyring service running. Press Enter to stop.");
    wait_for_enter().await;
}

/// Waits for Enter, end of input, Ctrl+C or SIGTERM, so the environment is
/// always dropped and its daemons stopped.
async fn wait_for_enter() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let enter = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
    });
    tokio::select! {
        _ = enter => {}
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
