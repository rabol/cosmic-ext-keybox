# MVP qualification status

Status of the acceptance criteria in [`specs.md`](../specs.md) §8, as of
2026-10-22 (commit after step 4). "Automated" means covered by `cargo test`
or `scripts/privacy-check.sh`; "screenshot" means checked with the debug
script runner (`src/ui/dev.rs`) against synthetic keyrings.

| # | Criterion | Status | Evidence |
| --- | --- | --- | --- |
| 1 | Launches from the COSMIC launcher, follows light/dark | **Partly** | Desktop entry and metainfo validate (`desktop-file-validate`, `appstreamcli`); install layout checked with `just rootdir=… install`. Launcher visibility after a real install and light-theme appearance not yet checked |
| 2 | oo7 and GNOME Keyring pass browse, unlock, reveal, copy and lock | **Done** for both | `tests/oo7.rs`, `tests/gnome_keyring.rs`, screenshots; versions in [`providers.md`](providers.md). GNOME Keyring unlock tested without a prompter only |
| 3 | Startup, search and refresh make no secret retrieval calls | **Done** | Bus-monitor tests in both suites; search works on loaded metadata only (`model` unit tests) |
| 4 | Locked, empty, inaccessible and failed states distinguishable | **Done** | `model::Load`, separate views for each; screenshots |
| 5 | Prompt completion, cancellation and service failure leave the UI usable | **Done** | Tests for completed, rejected and abandoned prompts and owner loss; screenshots |
| 6 | Selection change, expiry, deactivation and owner change remove secrets; late results can't reveal | **Done** | Operation IDs and generations; screenshots of each case. Real focus loss still to confirm manually (simulated in tests) |
| 7 | Clipboard expiry respects newer ownership, or the limitation is exposed | **Done**, with stated limits | Clipboard checks in [`providers.md`](providers.md); Settings explains focus dependence and clipboard managers |
| 8 | Duplicate labels, missing attributes, empty/invalid UTF-8/binary secrets, large metadata | **Done** | Fixtures in `tests/support`; both providers |
| 9 | External changes/deletion update the UI; reconnect doesn't reuse identities | **Done** | Event tests; `Stale` errors for old generations |
| 10 | Logs and preferences contain no fixture data | **Done** | `scripts/privacy-check.sh` (Keybox prints nothing; only `reveal_seconds` is saved) |
| 11 | Fully keyboard operable; accessible semantics; screen reader verified | **Partly** | Tab/Shift+Tab focus (libcosmic), arrow keys, shortcuts verified. AT-SPI exposure not observed; see [`providers.md`](providers.md#accessibility-at-spi) |

## Management release (specification §3, §9 step 5)

| Requirement | Status | Evidence |
| --- | --- | --- |
| Create text entries; edit labels, attributes and text secrets | **Done** for oo7 and GNOME Keyring | `tests/management.rs` (both providers); editor unit tests; flows driven with the debug script runner and checked on the test bus |
| Delete an entry with confirmation naming entry and keyring | **Done** | Dialog shows both names; tests for both providers |
| Create and rename keyrings | **Done** for oo7 (with a prompter); creation untested to success on GNOME Keyring | Fake KDE prompter answers oo7's `CreateCollectionPrompt`; on COSMIC the prompt never appears and Cancel is offered |
| Set the default alias | **Not offered** | Backend implemented and verified; oo7 0.6.0 is unreliable, see [`providers.md`](providers.md#changing-stored-data-management-release) |
| Delete empty keyrings only | **Done** for oo7 | Default and session keyrings excluded; emptiness rechecked before the call |
| Never replace an entry implicitly | **Done** | `replace=false` always; duplicate attributes need an explicit choice; tests on both providers |
| Re-read before editing; warn about external changes | **Done**, not atomic | Changes during editing are flagged; a re-read before saving shows conflicts and deletions |
| Secrets sent only encrypted | **Done** | Bus-monitor assertion in `tests/management.rs`; saving a secret fails without an encrypted session |
| No typed data in logs or preferences | **Done** | `scripts/privacy-check.sh` now creates, edits and deletes entries and keyrings |
| Visual check of the editor and dialogs | **Open** | The debug screenshots need the Keybox window to be visible on screen |

## Still open before an MVP release

- Install for real and check the COSMIC launcher, light theme and icon.
- Manual check of real focus loss (hide on deactivation, deferred clipboard clear).
- Screen-reader check with Orca; report upstream if libcosmic exposes nothing.
- GNOME Keyring unlock through a real prompter (needs GNOME Shell or gcr).
- KWallet and KeePassXC: untested.
- Prompt window parenting on Wayland: Keybox passes no window identifier yet.
- Session-lock notification: Keybox relies on window deactivation only.
- Push to GitHub and build the Arch package from it.
