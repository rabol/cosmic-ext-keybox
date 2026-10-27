# Changelog

All notable changes to Keybox. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-10-27

First release.

### Added

- Browse keyrings and their entries from any freedesktop Secret Service
  provider. Browsing, search and refresh read no secrets.
- Reveal and copy a selected secret. A revealed secret hides after a set time
  (30 seconds by default), when the selection changes, when the window loses
  focus, or when the entry or keyring changes. A copied secret is cleared from
  the clipboard after 30 seconds if it is still there.
- Escaped view for text with control characters; binary secrets are not
  displayed.
- Create and edit text entries (label, secret, attributes), with warnings for
  duplicate attributes and for changes made by other applications. Entries
  are never replaced implicitly.
- Delete entries, with a confirmation naming the entry and its keyring.
- Create, rename and delete keyrings. Only empty keyrings can be deleted, never
  the default or the session keyring.
- Lock and unlock keyrings.
- Built-in password prompt for Keybox's own unlock and create requests on
  desktops without a keyring prompter, such as COSMIC. Used only when no other
  prompter is available, and only for Keybox. Can be left out with
  `--no-default-features`.
- Keyboard shortcuts for all main actions; search over labels and attributes.
- Secrets travel only over an encrypted session, and Keybox wipes the memory
  that held them after use. Keybox keeps no logs and no data besides the
  reveal duration setting.
- Tested with oo7 0.6.0 and GNOME Keyring 51.1. See
  [`docs/providers.md`](docs/providers.md) and
  [`docs/qualification.md`](docs/qualification.md) for what is verified and
  the known limits.

[0.1.0]: https://github.com/rabol/cosmic-ext-keybox/releases/tag/v0.1.0
