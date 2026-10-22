# Keybox

A keyring manager for the COSMIC desktop. Keybox shows which application
credentials are stored in your desktop keyring and lets you inspect, reveal and
copy a selected entry.

Keybox is a client of the freedesktop Secret Service API
(`org.freedesktop.secrets`). It works with whichever provider is running
(oo7, GNOME Keyring, KWallet, KeePassXC, ...) and never reads keyring files
directly. It is not affiliated with System76.

**Status:** early development. The Secret Service backend is in place; the
COSMIC interface is not built yet. See [`specs.md`](specs.md) for the
specification and [`docs/providers.md`](docs/providers.md) for tested
providers.

## Development

Requirements: Rust 1.92+, and for the integration tests `dbus-daemon` and
`oo7-daemon`.

```sh
cargo test
cargo clippy --all-targets -- -W clippy::pedantic
```

The integration tests start a private D-Bus and a private `oo7-daemon` with
disposable keyrings in a temporary directory. They never touch your session
bus or your real keyrings. Tests are skipped if those programs are missing.

## License

GPL-3.0-only. See [`LICENSE`](LICENSE).
