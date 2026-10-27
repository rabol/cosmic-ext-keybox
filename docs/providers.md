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

The custom client uses the RustCrypto crates (`aes`, `cbc`, `hkdf`, `sha2`;
newer releases than oo7 0.6 since 2026-10-27), rejects plaintext
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
| Start-up (checked 2026-10-27) | Owns `org.freedesktop.secrets` **before** its keyrings are loaded. Until then `Collections` is empty, calls that create objects (prompts, sessions) panic the handler (`Service::connection` unset) so the caller times out, and reading `Collections` while it registers keyrings can deadlock. The tests wait for the `session` collection object (registered last) via introspection. Matters for Keybox only if it activates the daemon itself; at login the daemon is normally already running |

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

- An unlock request shows a "waiting for the keyring service" state with a
  Cancel action, and a hint if no prompt appears. Cancel drops the request,
  which dismisses the prompt.
- Since 2026-10-27 Keybox answers its own unlock and create requests with a
  built-in prompter (next section). Other applications still get no prompt,
  so the Lock action's confirmation says that only Keybox can unlock again
  on this desktop.

The integration tests use a fake KDE prompter to cover successful unlock,
retry, rejection and the no-answer case.

### Keybox's built-in prompter

Added 2026-10-27, behind the `builtin-prompter` Cargo feature (default on).
Upstream status checked the same day: no prompter is planned in any pop-os
repository; the open report is
[cosmic-epoch#3453](https://github.com/pop-os/cosmic-epoch/issues/3453).

On COSMIC, oo7 0.6 always uses the GNOME prompter protocol
(`org.gnome.keyring.internal.Prompter` on `org.gnome.keyring.SystemPrompter`,
the protocol `gcr-prompter` implements). While one of Keybox's unlock or
create-keyring requests is pending, Keybox
(`src/backend/prompter.rs`, `src/ui/prompter.rs`):

1. Checks that the provider is oo7 or GNOME Keyring and that nobody owns
   `org.gnome.keyring.SystemPrompter` and no service file provides it.
   Otherwise it steps aside and the real prompter is used.
2. Takes the name (`DoNotQueue`) on a separate connection and passes a random
   window identifier, `wayland:keybox-<128-bit hex>`, to the provider's
   `Prompt` call. oo7 forwards it as the prompt's `caller-window` (oo7 drops
   identifiers that are not `wayland:` or `x11:`, hence the form).
3. Accepts calls only from the keyring service's unique name, and answers only
   prompts whose `caller-window` is that token. Other applications' prompts
   get no answer, exactly as without Keybox.
4. Shows its own dialog and replies with the password encrypted by the
   `[sx-aes-1]` secret exchange: Diffie-Hellman in the RFC 3526 1536-bit
   group, HKDF-SHA256 of the shared secret padded to 192 bytes, AES-128-CBC.
   Typed text is kept in wiping buffers. A wrong password makes oo7 send the
   prompt again with a warning; Keybox shows it again.
5. Releases the name as soon as the request ends, is cancelled, or fails.

Verified against oo7 0.6.0 in `tests/builtin_prompter.rs`: unlock with a
wrong then right password, cancel, new keyring password, other applications'
prompts left alone, calls from other senders rejected, stepping aside for a
running or activatable prompter, name released afterwards, and no password
bytes in any `PromptReady` call on the bus.

Not tested: GNOME Keyring through this prompter (it uses the same protocol,
but whether it forwards `caller-window` is unchecked; if not, Keybox's
requests simply get no answer, as before).

To remove it once COSMIC has a prompter: delete the two files above and
`tests/builtin_prompter.rs`, then everything marked
`cfg(feature = "builtin-prompter")` and the feature in `Cargo.toml`.

## Changing stored data (management release)

Tested 2026-10-22 with `tests/management.rs` against oo7 0.6.0 and GNOME
Keyring 51.1, on private buses with disposable keyrings. A bus monitor
confirms that no secret plaintext appears in `CreateItem` or `SetSecret`
calls: Keybox sends secrets only over the encrypted session.

| Scenario | oo7 0.6.0 | GNOME Keyring 51.1 |
| --- | --- | --- |
| Create a text entry (`replace=false`) | Works without a prompt; adds `xdg:content-type` | Works without a prompt |
| Create again with the same attributes | Second, separate entry | Second, separate entry |
| `SearchItems` for those attributes | Finds both | Finds both |
| Set label, attributes, secret | Work; `ItemChanged` emitted | Work; setting attributes adds `xdg:schema` = `org.freedesktop.Secret.Generic` |
| Delete an entry | No prompt; `ItemDeleted` emitted | No prompt; `ItemDeleted` emitted |
| Read a deleted entry | `NoSuchObject` | Generic error; Keybox checks the keyring's entry list instead |
| Write to a locked keyring | Create and delete wait for an unlock prompt (on COSMIC answered only by Keybox's built-in prompter); set label fails with a generic error; set secret fails with `IsLocked` | `IsLocked` for all |
| `SearchItems` in a locked keyring | Finds nothing | Finds matches |
| Create a keyring | Asks the desktop prompter (`CreateCollectionPrompt`); works with one, waits forever without (COSMIC, unless Keybox's built-in prompter answers) | Dismissed at once without a prompter |
| Rename a keyring | Works (unlocked); generic error while locked | Works |
| Set the default alias | **Unreliable**: marks the target without unmarking the old default, and `ReadAlias` then returns whichever comes first. Aliases are kept in memory only | Works both ways |
| Delete an empty keyring | Works without a prompt | Not tested: no second keyring can be created without a prompter |
| Delete the `session` keyring | oo7 deletes it | Not tested |

Consequences for Keybox:

- Changes are offered only while the keyring (and entry) is unlocked, so no
  hidden unlock prompt is started. Every change runs with a "waiting for the
  keyring service" banner and Cancel.
- Before creating or editing, Keybox searches for other entries with the same
  attributes and asks before saving. It always creates with `replace=false`.
- Setting the default keyring is implemented and verified in the backend
  (it reads the alias back and reports when the service did not apply it),
  but not offered in the interface because of the oo7 behaviour above.
- Keybox never offers to delete the default or the session keyring, and
  checks again that a keyring is empty right before deleting it. The check and
  the deletion are separate calls; the service offers nothing atomic.
- Keybox does not promise atomic conflict detection: it re-reads an entry
  just before saving and warns about differences, but a change made between
  that read and the save can be overwritten.

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
limits. Clipboard managers may keep copies Keybox cannot remove: on the test
machine, a clipboard manager applet (`clipboard-manager-git`) is installed,
and secrets copied from Keybox should be assumed to land in its history.
The README tells users about this.

The unfocused case was exercised by simulating focus loss inside Keybox;
real focus changes still need a manual check.

## GNOME Keyring 51.1 (Arch package `gnome-keyring 1:51.1-1`)

Tested 2026-10-22 against an isolated `gnome-keyring-daemon --foreground
--components=secrets --unlock` on a private bus (`tests/gnome_keyring.rs`).
On Arch, the `oo7` package conflicts with `gnome-keyring`, so the daemon was
run from the signature-verified package unpacked into a temporary directory
(`KEYBOX_TEST_GNOME_KEYRING_ROOT`), without installing it.

| Scenario | Result |
| --- | --- |
| Encrypted session | Works (also validates Keybox's own key exchange) |
| Provider detection (`gnome-keyring-d`) | Works |
| Default alias | `login` |
| `session` collection | Present, with an empty label (shown as "Untitled keyring") |
| Content types | Reports `text/plain` for every secret, including binary data and items stored as `text/plain; charset=utf8`. Keybox decides text vs. binary from the bytes |
| Lock | Completes without a prompt; emits `CollectionChanged` |
| Locked collection metadata | Labels and attributes **stay visible** while locked (unlike oo7) |
| Unlock without a prompter | Prompt completes as dismissed immediately (oo7 waits forever) |
| Item order | Not sorted; Keybox sorts by label |
| Daemon stop | Owner change observed; old paths unusable |

Keybox cannot tell "the user cancelled" from "no prompter available" with
GNOME Keyring; the message after a dismissed unlock covers both.

## Accessibility (AT-SPI)

Checked 2026-10-22 on COSMIC with libcosmic `430f3d4` (built with its `a11y`
feature). With `org.a11y.Status.IsEnabled` switched on, Keybox did **not**
appear on the AT-SPI bus. Neither did other running COSMIC applications
(cosmic-term, cosmic-settings), although those were started before
accessibility was enabled. Screen-reader support is therefore unverified and
should be rechecked with Orca. Keyboard operation was verified separately.

## KWallet / KDE Secret Service implementation

Not yet tested.

## KeePassXC

Not yet tested.
