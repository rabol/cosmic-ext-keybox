# Keybox

A keyring manager for the COSMIC desktop. Keybox shows which application
credentials are stored in your desktop keyring and lets you inspect, reveal and
copy a selected entry.

Keybox is a client of the freedesktop Secret Service API
(`org.freedesktop.secrets`). It works with whichever provider is running
(oo7, GNOME Keyring, KWallet, KeePassXC, ...) and never reads keyring files
directly. It is not affiliated with System76.

**Status:** early development. Browsing keyrings and entries, search, and
locking/unlocking work. Revealing and copying secrets is next. See [`specs.md`](specs.md) for the
specification and [`docs/providers.md`](docs/providers.md) for tested
providers.

## Development

Requirements: Rust 1.92+, [`just`](https://github.com/casey/just), and for the
integration tests `dbus-daemon` and `oo7-daemon`.

```sh
just check              # fmt, clippy (pedantic) and tests
just build-release
sudo just install       # binary, desktop entry, metainfo and icon
```

### Trying Keybox without your real keyring

```sh
just dev-env            # prints KEYBOX_DEV_BUS=...
KEYBOX_DEV_BUS=... cargo run
```

`dev-env` starts a private bus with a disposable oo7-daemon, synthetic
keyrings and a fake unlock prompter. `KEYBOX_DEV_BUS` and `KEYBOX_DEV_SCRIPT`
(scripted steps and screenshots of Keybox's own window, see `src/ui/dev.rs`)
only exist in debug builds.

The integration tests start a private D-Bus and a private `oo7-daemon` with
disposable keyrings in a temporary directory. They never touch your session
bus or your real keyrings. Tests are skipped if those programs are missing.

## License

GPL-3.0-only. See [`LICENSE`](LICENSE).
