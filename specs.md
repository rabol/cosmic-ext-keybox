# Keybox — application specification

Status: proposed specification; no application implemented.
Date: 2026-10-22.
Suggested name: **Keybox** — Keyring Manager for COSMIC. Name availability, final name, application ID and license must be confirmed before publication. The name must not imply official System76 ownership.

## 1. Purpose

Build a native COSMIC desktop application for inspecting and managing secrets exposed by the user's desktop keyring service. The initial release focuses on browsing, unlocking, revealing and copying existing entries. Support multiple providers through a common protocol rather than tying the interface to oo7.

The primary use case is: “Show me which application credentials are stored in my desktop keyring and let me inspect a selected entry.”

Use Rust and libcosmic. Target Linux with COSMIC on Wayland, with Arch Linux as the initial development environment. Prefer native distribution packaging and avoid requiring GTK, Qt, GNOME Shell or KDE Plasma for the application interface. A provider may independently use another toolkit for its authentication dialog.

## 2. Protocol and provider scope

The integration boundary is the **freedesktop Secret Service API**, using the session D-Bus name `org.freedesktop.secrets`. Its model consists of collections, items, sessions and prompts. Items carry labels, lookup attributes and secret bytes. Collections are presented as “Keyrings” in the UI. Unlocking and authentication remain service responsibilities. [1]

The standard endpoint has one active owner on a session bus. Multiple installed providers do not imply multiple simultaneously browsable stores. The application connects to the active owner; it must not offer a misleading provider switcher or start competing providers. Custom endpoints and direct file adapters are outside the initial release.

Compatibility targets:

| Provider | Intended support | Release qualification |
| --- | --- | --- |
| oo7 | Standard Secret Service endpoint | Required integration target |
| GNOME Keyring | Standard Secret Service endpoint | Required integration target |
| KWallet / KDE Secret Service implementation | Standard endpoint where available and enabled | Test the actual version and configuration; no assumption that every KWallet installation exposes it |
| KeePassXC | Entries exposed through its enabled Secret Service integration | Additional target; do not imply access to the entire database |
| Other providers | Same protocol adapter | Best effort until tested |

This table is a testing plan, not a claim of verified application compatibility. Record provider versions and observed differences in the eventual project documentation.

Only service-exposed entries are visible. Provider-private data, SSH agents, OpenPGP keys, browser password databases and arbitrary vault files are outside scope. “No entries exposed” must never be presented as proof that a provider has no stored data.

### Alternatives considered

- A provider-specific file reader would need storage-format, encryption and locking logic for every implementation. Reject for the initial product.
- Wrapping command-line tools would add runtime dependencies and complicate structured errors and secret handling. Prefer a Rust protocol client.
- A GTK/libsecret application is possible, but libcosmic better meets the native COSMIC interface goal.
- Building a new keyring daemon or password vault would duplicate storage and authentication responsibilities. Keep this application a client.

## 3. Release boundaries

### MVP: inspection

- Connect to the active Secret Service, with explicit connection and retry states.
- List accessible keyrings and entries; show locked state and available default-keyring information.
- Unlock and lock selected objects through service requests.
- Search loaded metadata without reading secret values.
- Inspect entry details and explicitly reveal or copy textual secrets.
- Handle service changes and external entry updates without exposing stale secrets.
- Provide native appearance, keyboard navigation, desktop integration and actionable errors.

The MVP must not create, modify or delete stored entries. Locking and unlocking are deliberate service-state changes and remain available.

### Subsequent management release

- Create text entries and edit labels, attributes and text secrets where supported.
- Delete a selected entry with explicit confirmation identifying the entry and keyring.
- Create and rename collections, set the default alias and delete empty collections only after provider testing.
- Never replace an existing entry implicitly because its attributes match. Make replacement a separate, explicit user action.
- Re-read state before editing; warn about detected external changes. Do not promise atomic conflict detection where the service cannot provide it.

Defer collection password changes, migration, bulk operations, import/export, binary secret editing and nonempty collection deletion. The common API does not provide a portable collection-password-change workflow. [1]

## 4. User experience

### Main window

Use a COSMIC header bar with application menu, search, refresh and connection status. Use a navigation sidebar for keyrings, an entry list and a details pane. Collapse into navigable views at narrow widths; do not require a large display.

The entry list shows label and lock state. Show a neutral fallback for missing labels. Sort by label, with a stable secondary identity. Do not treat identical labels as the same entry.

The details pane shows label, keyring, attributes, lock state, content type and timestamps when supplied. Raw service identity and object paths belong in expandable technical details. Treat all labels and attributes as untrusted text: no markup rendering, automatic URL opening or shell interpretation.

Search is case-insensitive over loaded labels and attributes. It does not retrieve secrets, unlock keyrings automatically or claim to search undisclosed metadata. Show a notice when locked or inaccessible content limits results. Attribute values are metadata but may still be sensitive; never include them in telemetry or logs.

### Reveal and copy

1. The user selects an entry; no secret is fetched yet.
2. The user chooses Reveal or Copy secret.
3. If necessary, request unlocking and let the provider handle its prompt.
4. Retrieve only the selected secret. Display valid textual content without lossy conversion; identify unsupported binary content without automatically dumping it.
5. Reveal expires after 30 seconds by default, and immediately on selection change, navigation away, window deactivation or close.

Provide an explicit Hide action. Copy may fetch a secret without revealing it. Copy notifications contain no secret value. Text containing embedded NUL or control characters needs an escaped inspection view; copying must preserve the original text and require a clear action. Invalid UTF-8 and binary values receive a “Binary content; viewing is not supported in this release” state. Never reinterpret an error as an empty secret.

Clear a copied secret after 30 seconds on a best-effort basis only if the application still owns that clipboard content. Do not overwrite newer clipboard content. Validate this behavior on COSMIC Wayland before promising it. If reliable ownership-aware clearing is unavailable, say so in settings/help and do not use unsafe unconditional clearing. Clipboard managers and other applications may retain copies.

### Authentication and locking

Provider prompts are external UI; do not collect a master password in an application-owned dialog. Handle prompt cancellation as a normal outcome. Parent prompts to the window when a supported window identifier is available; validate Wayland behavior rather than assuming X11 identifiers work.

Distinguish hiding a displayed secret from locking a keyring. A lock action may affect other applications, so label it explicitly. After a lock request, refresh actual state and report a provider that leaves the collection unlocked. Closing the viewer must not automatically lock shared keyrings.

### Connection states

Distinguish loading, connected, no service available, activation failed, permission denied, prompt pending, prompt cancelled, disconnected, and unsupported operation. Retry is explicit after failures. Initial connection may use normal D-Bus activation of the configured service; the app must not install, enable or choose a provider.

Show the connected service and, when reliably detected, its provider name. Otherwise display “Secret Service” rather than guessing from installed packages. Never report an empty keyring while a request is still loading or has failed.

## 5. Architecture

Separate UI, application state and protocol access:

- `ui`: libcosmic views, actions, localization and accessibility.
- `model`: collection/item metadata, connection state, selection and operation state.
- `backend`: asynchronous Secret Service adapter with typed errors and provider-neutral results.
- `secret`: narrowly scoped secret buffers and reveal/copy lifetime handling.
- `settings`: non-secret preferences only.

Use the official COSMIC application template as a starting point and pin a known libcosmic revision plus Cargo.lock for reproducible application builds. [2][3]

Evaluate the Rust `secret-service` crate first; it exposes a protocol client including encrypted-session negotiation. [4] An explicit D-Bus adapter from oo7 is another candidate. Do not use an auto-selected file/portal backend for browsing host keyrings: the oo7 library distinguishes those backends, and its convenience API may select between them. [5] Select the dependency after a small compatibility spike, not from its package name.

Keep D-Bus operations asynchronous and off blocking UI paths. Track an operation ID and connection generation. Discard late results when the selection or service owner changes. Secret values must not appear in generic UI messages with derived Debug output or in general metadata caches.

Identify an object by connection generation and D-Bus object path, never its label. Paths from a previous owner must be invalidated. On service loss, immediately hide and release displayed secrets, invalidate selections, and offer reconnect.

Subscribe to applicable service, collection, item and property change signals and bus owner changes. Refresh after operations and provide manual refresh as a fallback. Do not fetch all secret values during startup, refresh or signal handling.

Represent unknown or untested operation support separately from supported and unsupported. Do not infer support from a provider name. Introspection alone does not prove that an operation will succeed. Prefer safe reads; do not probe support by changing real user data.

## 6. Security and privacy requirements

- Store no secrets, search history, entry metadata or master passwords on disk. Persist only UI preferences such as window size and reveal duration.
- No analytics, network requests, crash uploads or automatic update checks in the application.
- No secret, label, attribute, clipboard or prompt content in logs. Diagnostic output may include sanitized operation names and error categories. Treat service error text as potentially sensitive.
- Prefer encrypted D-Bus secret-transfer negotiation. If unavailable, fail the secret operation with an explanatory message in the initial release; never silently fall back to plaintext transfer. This is transport handling, not a claim of protection from a compromised session.
- Minimize secret copies, use zeroizing buffers where practical, and redact secret wrappers from Debug/Display. Document unavoidable copies in GUI, clipboard and dependency code; do not promise complete memory erasure.
- Hiding or releasing a local value does not remove copies held by the provider or other processes. The app does not add per-application isolation to Secret Service.
- Cancel or ignore stale operations, release buffers on expiry, and recover without panic when objects disappear.
- On session lock, hide and release displayed secrets when a reliable lock notification is available. Verify COSMIC integration; window-deactivation hiding is the baseline, not a substitute for claiming session-lock support.
- No elevated permissions, direct keyring-file access, PAM changes or daemon replacement.

## 7. Packaging and desktop integration

Provide a native executable, icon, desktop entry, application metadata and license. The display name should be discoverable with search terms including keyring, password, secret and credentials. Verify launcher visibility in COSMIC after installation.

The package depends on a compatible running Secret Service provider without forcing oo7, GNOME Keyring or KWallet specifically. Do not bundle or register a competing `org.freedesktop.secrets` service.

Flatpak is a later packaging target. The Secret portal does not grant a general keyring-browser interface; host Secret Service access requires explicitly evaluated D-Bus permissions. [5] Do not present sandbox packaging as an equivalent tested implementation before validating access and prompts.

## 8. Verification and acceptance

Use mock backend tests for UI state and an isolated session bus with disposable test data for protocol integration. Never run mutation tests against the developer's real keyrings. Test providers one at a time.

The MVP is accepted when:

1. Clean installations launch from the COSMIC launcher and follow system light/dark appearance.
2. oo7 and GNOME Keyring each pass the same browse, unlock, reveal, copy and lock scenarios; additional providers have documented results and versions.
3. Startup, metadata search and refresh make no secret retrieval calls.
4. Locked, empty, inaccessible and failed states remain distinguishable.
5. Prompt completion, cancellation and service failure leave the interface usable.
6. Selection changes, reveal expiry, window deactivation and service-owner changes remove visible secrets; delayed results cannot reveal a previous selection.
7. Clipboard expiry respects newer clipboard ownership, or the limitation is honestly exposed.
8. Duplicate labels, missing attributes, empty secrets, invalid UTF-8, large metadata and binary secrets are handled without crash or accidental disclosure.
9. External item changes and deletion update the UI; reconnection does not reuse old object identities.
10. Logs and saved preferences contain none of the distinctive synthetic secret or metadata fixtures.
11. The application can be fully operated by keyboard; focus, button names, errors and reveal state have accessible semantics. Verify actual screen-reader support and document any toolkit limitation.

Before each release, run formatting, linting, targeted tests, integration scenarios and a manual COSMIC Wayland check. Document untested providers and behavior explicitly.

## 9. Implementation sequence

1. **Compatibility spike:** connect to disposable providers, enumerate metadata, handle prompts, test encrypted sessions, confirm lock behavior and assess the Rust client dependency.
2. **Native shell:** desktop integration, three-pane/adaptive layout, asynchronous connection states and metadata browsing.
3. **Secret inspection:** scoped retrieval, reveal lifetime, ownership-aware clipboard handling, errors and service restart recovery.
4. **MVP qualification:** provider matrix, accessibility, packaging, synthetic-data privacy checks and user documentation.
5. **Management release:** implement explicitly authorized mutations with confirmations and separate integration coverage.

## 10. References

Checked 2026-10-22. Upstream APIs and provider behavior must be rechecked when implementation starts. The specification linked below currently labels itself a draft; target interoperable behavior observed in released providers rather than assuming every draft feature is implemented.

1. [freedesktop Secret Service API](https://specifications.freedesktop.org/secret-service/latest-single/) — common model, D-Bus interfaces and protocol limitations.
2. [libcosmic](https://github.com/pop-os/libcosmic) — native COSMIC toolkit.
3. [COSMIC application template](https://github.com/pop-os/cosmic-app-template) — project starting point.
4. [Rust secret-service crate](https://docs.rs/secret-service/latest/secret_service/) — candidate protocol client.
5. [oo7 library documentation](https://linux-credentials.github.io/oo7/oo7/) — explicit D-Bus versus file/portal backend distinction.
