app-title = Keybox
app-comment = Inspect the passwords and secrets stored in your desktop keyring
app-keywords = keyring;password;passwords;secret;secrets;credentials;keychain;wallet;

## Menus
menu-file = File
menu-view = View
menu-keyring = Keyring
about = About Keybox
quit = Quit
refresh = Refresh
search = Search
repository = Repository
lock-keyring = Lock keyring…
unlock-keyring = Unlock keyring

## Header and connection status
status-connecting = Connecting…
status-connected = Connected to { $provider }
status-disconnected = Not connected
secret-service = Secret Service

## Connection states (full-page)
connecting-title = Connecting to the keyring service
no-service-title = No keyring service is available
no-service-body = Keybox needs a Secret Service provider, such as oo7 or GNOME Keyring, running in your session. Keybox does not install or choose one for you.
activation-failed-title = The keyring service could not be started
activation-failed-body = Your session has a keyring service configured, but it failed to start. Check that it is installed correctly, then try again.
permission-denied-title = Access to the keyring service was denied
permission-denied-body = The keyring service refused the connection.
connection-lost-title = The keyring service went away
connection-lost-body = The keyring service stopped or was replaced. Entries from the old connection are no longer shown.
connection-failed-title = Could not connect to the keyring service
connection-failed-body = { $reason }
retry = Try again
reconnect = Reconnect

## Keyrings
keyring-untitled = Untitled keyring
keyring-default = Default keyring
keyring-locked = Locked
keyring-unlocked = Unlocked
keyring-unavailable = Unavailable
keyrings-loading = Loading keyrings…
keyrings-failed = Could not list keyrings: { $reason }
keyrings-none = The keyring service does not expose any keyrings.
select-keyring = Select a keyring

## Entries
entries-loading = Loading entries…
entries-failed = Could not list entries: { $reason }
entries-none = The keyring service exposes no entries in this keyring.
entries-none-note = This does not mean nothing is stored: a provider may keep data it does not expose.
entries-locked = This keyring is locked
entries-locked-body = Unlock it to see its entries. The keyring service will ask for its password.
entries-hidden = { $count ->
    [one] One entry is hidden until this keyring is unlocked.
   *[other] { $count } entries are hidden until this keyring is unlocked.
}
entry-untitled = Untitled entry
entry-locked = Locked entry
select-entry = Select an entry to see its details

## Search
search-placeholder = Search labels and attributes
search-results = { $count ->
    [0] No matching entries
    [one] One matching entry
   *[other] { $count } matching entries
}
search-limited = Some content could not be searched: { $reasons }
search-limited-locked = { $count ->
    [one] one entry in a locked keyring
   *[other] { $count } entries in locked keyrings
}
search-limited-failed = { $count ->
    [one] one keyring failed to load
   *[other] { $count } keyrings failed to load
}
search-limited-loading = { $count ->
    [one] one keyring is still loading
   *[other] { $count } keyrings are still loading
}

## Details
back = Back
details-keyring = Keyring
details-status = Status
details-created = Created
details-modified = Modified
details-content-type = Content type
details-attributes = Attributes
details-no-attributes = No attributes
details-attributes-hidden = Not disclosed while locked
details-technical = Technical details
details-object-path = Object path
details-keyring-path = Keyring object path
details-label-shortened = Label shortened; it has { $count } characters.
details-unavailable = Could not read this entry: { $reason }

## Lock and unlock
lock-dialog-title = Lock “{ $keyring }”?
lock-dialog-body = Apps that use this keyring will not be able to read their passwords until it is unlocked again. Unlocking needs the keyring service to show a password prompt, which not every desktop provides.
lock-dialog-confirm = Lock keyring
cancel = Cancel
unlock-waiting = Waiting for the keyring service to ask for the password…
unlock-waiting-hint = No password prompt appeared? Your desktop may not provide one for this keyring service. You can cancel and try again later.
unlock-cancelled = Unlocking was cancelled.
lock-still-unlocked = The keyring service left “{ $keyring }” unlocked.
operation-unsupported = The keyring service does not support this.
operation-failed = The operation failed: { $reason }

## Error reasons
error-no-service = no keyring service is available
error-activation = the keyring service could not be started
error-permission = permission denied
error-encryption = the keyring service does not support encrypted transfer
error-locked = the keyring is locked
error-no-object = it no longer exists
error-stale = it belongs to an earlier connection
error-dismissed = the prompt was dismissed
error-disconnected = the connection was lost
error-unsupported = not supported by the keyring service
error-timeout = the keyring service did not respond in time
error-session = the secure session expired
error-protocol = the keyring service sent an unexpected reply
error-service = the keyring service reported an error ({ $name })
error-crypto = decryption failed

## Secret
secret-title = Secret
secret-hidden = Hidden. Nothing is retrieved until you choose Reveal or Copy.
secret-retrieving = Retrieving…
secret-reveal = Reveal
secret-hide = Hide
secret-copy = Copy
secret-copy-exact = Copy exact text
secret-empty = This secret is empty.
secret-control-note = Contains line breaks or other control characters, shown here as escapes. Copy uses the exact stored text.
secret-binary = Binary content ({ $bytes } bytes); viewing is not supported in this release.
secret-binary-no-copy = Binary content can't be copied in this release.
secret-hides-in = Hides in { $seconds } s · { $content_type }
secret-unknown-type = unknown type
secret-failed = Could not retrieve the secret: { $reason }
secret-copied = Secret copied. Keybox will clear it from the clipboard after { $seconds } seconds unless something else was copied since.
secret-clipboard-cleared = The copied secret was cleared from the clipboard.
copy-control-title = Copy text with hidden characters?
copy-control-body = This secret contains line breaks or other control characters. Keybox will copy it exactly as stored, including those characters.
copy-control-confirm = Copy exact text

## Settings
settings = Settings
settings-menu = Settings…
settings-secrets = Secrets
settings-reveal-duration = Hide revealed secrets after
settings-seconds = { $seconds } seconds
settings-clipboard = Clipboard
settings-clipboard-body = After you copy a secret, Keybox clears it from the clipboard after { $seconds } seconds, but only if the clipboard still holds exactly that secret. Keybox can only check the clipboard while its window is focused: if you switch away, the check happens when you return to Keybox. Clipboard managers and other apps may keep their own copies, which Keybox cannot remove.
