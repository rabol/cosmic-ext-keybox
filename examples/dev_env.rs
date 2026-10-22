// SPDX-License-Identifier: GPL-3.0-only

//! Starts the isolated test environment (private bus + oo7-daemon with
//! synthetic keyrings and a fake prompter) for manual testing, then waits.
//!
//! ```sh
//! cargo run --example dev_env            # prints KEYBOX_DEV_BUS=...
//! KEYBOX_DEV_BUS=... cargo run           # in another terminal
//! ```
//!
//! The fake prompter accepts the Work keyring password. Press Enter to stop.

#[path = "../tests/support/mod.rs"]
mod support;

#[tokio::main]
async fn main() {
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
    let _ = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
    })
    .await;
}
