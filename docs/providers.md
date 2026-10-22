# Provider compatibility notes

Results of the compatibility spike (specification §9 step 1). Record the
provider version and the date with every observation. Untested providers are
listed as such; nothing here is a compatibility claim beyond what was tested.

## Client library decision (2026-10-22)

Keybox talks to the Secret Service through its own small client on `zbus`
(`src/backend/`), with its own transfer-encryption module. Both candidate
libraries were evaluated first:

| Requirement | `secret-service` 5.2.0 | `oo7` 0.6.0 (`dbus` module) |
| --- | --- | --- |
| Encrypted session | Yes | Yes, but `Service::new()` silently falls back to plaintext |
| Change signals (item/collection created/changed/deleted) | No | Yes |
| Pass a window identifier to prompts | No (always empty) | Yes |
| Tolerant content types (e.g. `text/plain; charset=utf8`) | — | No: any other value fails deserialisation |
| Item access by object path | — | Only with the `unstable` feature |
| Plaintext wiped after decryption | — | No: one intermediate copy is not wiped |
| Malformed IV from the service | — | Panics (`expect("Invalid key length")`) |

The custom client uses the same RustCrypto crates as oo7, rejects plaintext
sessions, validates the service's public key and the IV, wipes every plaintext
buffer, and addresses all calls to the owner's unique bus name so a replaced
service never receives requests meant for the previous one. `oo7` is still used
by the tests to write disposable keyring files.

## oo7 0.6.0 (Arch package `oo7 0.6.0-5`)

Tested 2026-10-22 against an isolated `oo7-daemon` (private bus, temporary
data directories; see `tests/support/mod.rs`).

| Scenario | Result |
| --- | --- |
| Encrypted session (`dh-ietf1024-sha256-aes128-cbc-pkcs7`) | Works |
| Provider detection (`/proc/<pid>/comm` = `oo7-daemon`) | Works |
| Collections, `default` alias | Works. `Login` is the default; a `session` collection always exists |
| Locked collection metadata | Items are listed; `Label` and `Attributes` are not disclosed |
| Content types | Only `text/plain` and `application/octet-stream` are produced |
| Extra attributes | oo7 adds `xdg:content-type` to every item |
| Lock | Completes without a prompt; emits `CollectionChanged`, `ItemChanged` for every item and `PropertiesChanged` |
| Unlock | Always needs a prompt. Wrong passwords are retried inside the same prompt |
| Prompt dismissal | `Completed(dismissed=true)` |
| External label change / deletion | `ItemChanged` / `ItemDeleted` emitted; deleted paths return `NoSuchObject` |
| Item deletion by another client | Needs no prompt |
| Daemon restart | Name owner change observed; old paths unusable |

### Unlock prompts on COSMIC

`oo7-daemon` does not draw its own unlock dialog. It asks the desktop for one,
through GNOME Shell's `org.gnome.keyring.SystemPrompter`, or KDE's
`org.kde.secretprompter` when `XDG_CURRENT_DESKTOP=KDE` and that name is
activatable. COSMIC (checked 2026-10-22) provides neither. In that case an
`Unlock` call returns a prompt that **never completes and never fails**.

At login, `pam_oo7` normally unlocks the login keyring, so this mostly affects
keyrings that were locked later (for example through Keybox's Lock action) or
additional keyrings with their own password.

Consequences for Keybox:

- An unlock request must show a "waiting for the keyring service" state with
  a Cancel action, and a hint if no prompt appears. Cancel drops the request,
  which dismisses the prompt.
- The Lock action's confirmation should warn that unlocking again may not be
  possible from this desktop with the current provider.

The integration tests use a fake KDE prompter to cover successful unlock,
retry, rejection and the no-answer case.

## Clipboard on COSMIC Wayland

Tested 2026-10-22 on COSMIC (cosmic-comp, Arch) with libcosmic `430f3d4`,
which uses `window_clipboard` over the standard Wayland data device.

| Scenario | Result |
| --- | --- |
| Copy text, including newlines and a NUL character | Clipboard holds the exact text |
| Clear after 30 s while focused, clipboard unchanged | Cleared |
| Clear after 30 s while focused, other text copied since | Left alone |
| Timer runs out while Keybox is unfocused | Check waits; cleared on next focus if still ours |

Keybox gets no notification when another client takes the selection, and a
Wayland client can only read the selection reliably while focused. So
clearing is best effort: Keybox keeps a salted hash of what it copied (not
the text), reads the clipboard when the timer has run out and the window is
focused, and clears it only on a match. The Settings page states these
limits. Clipboard managers may keep copies Keybox cannot remove.

The unfocused case was exercised by simulating focus loss inside Keybox;
real focus changes still need a manual check.

## GNOME Keyring

Not yet tested. Required integration target before the MVP.

## KWallet / KDE Secret Service implementation

Not yet tested.

## KeePassXC

Not yet tested.
