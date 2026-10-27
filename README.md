# Keybox

A keyring manager for the COSMIC desktop. Keybox shows which application
credentials are stored in your desktop keyring and lets you inspect, reveal and
copy a selected entry, create and edit text entries, delete entries, and
create, rename or delete empty keyrings.

Keybox is a client of the freedesktop Secret Service API
(`org.freedesktop.secrets`). It works with whichever provider is running
(oo7, GNOME Keyring, KWallet, KeePassXC, ...) and never reads keyring files
directly. It is not affiliated with System76.

**Status:** 0.1.0, the first release. Tested with oo7 0.6 and GNOME Keyring
51.1; see [`docs/qualification.md`](docs/qualification.md) for what is
verified and what is still open. See [`specs.md`](specs.md) for the
specification, [`docs/providers.md`](docs/providers.md) for tested providers
and [`CHANGELOG.md`](CHANGELOG.md) for changes.

> **Note:** COSMIC has no keyring password prompt yet, so Keybox shows its own
> when it unlocks or creates a keyring. It is used only for Keybox's own
> requests and will be removed once COSMIC provides a prompt. See
> [Password prompt on COSMIC](#password-prompt-on-cosmic).

## Using Keybox

- Pick a keyring in the sidebar and an entry in the list. Nothing secret is
  read until you choose **Reveal** or **Copy**.
- A revealed secret hides itself after 30 seconds (see Settings), and as soon
  as you select something else, switch windows, or the entry or keyring
  changes.
- **Copy** puts the secret on the clipboard and clears it after 30 seconds if
  it is still there. Keybox can only check while its window is focused, and
  clipboard managers may keep their own copy.
- **Lock keyring** locks it for every app. Unlocking normally uses the
  desktop's keyring password prompt. COSMIC has none yet, so Keybox then asks
  for the password itself (see below). Other apps still cannot unlock a
  keyring on COSMIC; they wait for a prompt that never appears.

### Changing entries and keyrings

- **New entry** (in an unlocked keyring) and **Edit** open a form for the
  label, the secret and the attributes. Editing keeps the current secret
  unless you tick **Replace the secret**; Keybox never loads it into the form.
- Applications find their entries by attributes. Before saving, Keybox looks
  for other entries with the same attributes and warns you. It never replaces
  an existing entry; to change one, edit it.
- Before saving an edit, Keybox reads the entry again and warns you if another
  application changed it in the meantime. It cannot stop a change made at the
  very same moment.
- **Delete** asks for confirmation naming the entry and its keyring.
- The **Keyring** menu creates, renames and deletes keyrings. Only empty
  keyrings can be deleted, never the default or the session keyring. Creating
  a keyring asks for its password the same way as unlocking.
- Changing which keyring is the default is not offered: oo7 0.6 does not do it
  reliably (see [`docs/providers.md`](docs/providers.md)).
- Entries and keyrings must be unlocked before they can be changed.

### Password prompt on COSMIC

Keyring services (oo7, GNOME Keyring) do not draw password dialogs
themselves; they ask the desktop's prompter. COSMIC does not provide one yet,
so Keybox includes a stand-in that is used **only for Keybox's own unlock and
create requests**, and only when no other prompter is installed or running.
The password goes from Keybox's dialog to the keyring service encrypted, and
is not kept. Details: [`docs/providers.md`](docs/providers.md#keyboxs-built-in-prompter).

If another prompter is installed, Keybox uses that one instead. In particular
`gcr-prompter` (Arch package `gcr`, pulled in by `gnome-keyring` and
`seahorse`) counts, even though it may not appear on COSMIC
([cosmic-epoch#3453](https://github.com/pop-os/cosmic-epoch/issues/3453)).
If unlocking waits with no dialog while you use oo7, check whether `gcr` is
installed.

Other applications are not helped by this: on COSMIC they still cannot unlock
a keyring themselves. Unlock it in Keybox first.

It is temporary and sits behind the `builtin-prompter` Cargo feature (on by
default). Build with `--no-default-features` to leave it out. It will be
removed once COSMIC ships a prompter; the removal steps are in
[`docs/providers.md`](docs/providers.md#keyboxs-built-in-prompter).

### Keyboard

| Keys | Action |
| --- | --- |
| Up / Down, Home / End | Move through entries |
| Ctrl+Page Up / Page Down | Previous / next keyring |
| Ctrl+Shift+E | Reveal or hide the secret |
| Ctrl+Shift+C | Copy the secret |
| Ctrl+N | New entry |
| F2 | Edit the entry |
| Delete | Delete the entry (asks first) |
| Ctrl+F | Search labels and attributes |
| Ctrl+R, F5 | Refresh |
| Ctrl+, | Settings |
| Tab / Shift+Tab | Move focus |
| Escape | Close search or dialog, go back, leave an unchanged form |
| Ctrl+Q | Quit |

### Clipboard managers

If you use a clipboard manager, such as the COSMIC clipboard manager applet,
it records copied secrets in its history. Keybox only clears the system
clipboard; it cannot remove entries from a clipboard manager's history, and
those may be kept on disk. If that matters to you:

- prefer **Reveal** over **Copy**, or
- clear the secret from the clipboard manager's history after pasting, or
- check whether your clipboard manager can ignore or expire sensitive entries.

### Privacy

Keybox stores no secrets, labels, attributes or search text, makes no network
requests and writes no logs. Its only saved setting is the reveal duration.
It never falls back to unencrypted secret transfer, in either direction. A
secret you type is held in memory that is wiped after use, except for the
copy kept by the toolkit's text field, which Keybox cannot wipe. See
[`docs/qualification.md`](docs/qualification.md) for what has been verified.

## Development

Requirements: Rust 1.93+, [`just`](https://github.com/casey/just), and for the
integration tests `dbus-daemon` and `oo7-daemon`.

```sh
just check              # fmt, clippy (pedantic) and tests
just privacy-check      # opens Keybox on synthetic data; checks for leaks
just build-release
sudo just install       # binary, desktop entry, metainfo and icon
```

Arch package recipes are in [`packaging/arch`](packaging/arch): `cosmic-ext-keybox`
builds the tagged release, `cosmic-ext-keybox-git` the latest commit.

GNOME Keyring tests run when `gnome-keyring-daemon` is installed, or when
`KEYBOX_TEST_GNOME_KEYRING_ROOT` points at an unpacked `gnome-keyring`
package (on Arch it conflicts with `oo7`, so it need not be installed).

### Trying Keybox without your real keyring

```sh
just dev-env            # prints KEYBOX_DEV_BUS=...
KEYBOX_DEV_BUS=... cargo run
```

`dev-env` starts a private bus with a disposable oo7-daemon, synthetic
keyrings and a fake unlock prompter (`cargo run --example dev_env none`
leaves the prompter out, like COSMIC, so Keybox's own prompt is used). `KEYBOX_DEV_BUS` and `KEYBOX_DEV_SCRIPT`
(scripted steps and screenshots of Keybox's own window, see `src/ui/dev.rs`)
only exist in debug builds.

The integration tests start a private D-Bus and a private `oo7-daemon` with
disposable keyrings in a temporary directory. They never touch your session
bus or your real keyrings. Tests are skipped if those programs are missing.

## License

GPL-3.0-only. See [`LICENSE`](LICENSE).
