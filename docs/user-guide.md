# Keybox user guide

This guide covers Keybox 0.1.0: what each part of the window does, how to
work with entries and keyrings, and what Keybox's messages mean.

- [What Keybox is](#what-keybox-is)
- [Before you start](#before-you-start)
- [The window](#the-window)
- [Browsing and searching](#browsing-and-searching)
- [Entry details](#entry-details)
- [Revealing and copying secrets](#revealing-and-copying-secrets)
- [Locking and unlocking keyrings](#locking-and-unlocking-keyrings)
- [The built-in password prompt](#the-built-in-password-prompt)
- [Creating and editing entries](#creating-and-editing-entries)
- [Deleting entries](#deleting-entries)
- [Managing keyrings](#managing-keyrings)
- [Settings](#settings)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Connection problems](#connection-problems)
- [Messages and what they mean](#messages-and-what-they-mean)
- [Privacy and security](#privacy-and-security)
- [Known limits](#known-limits)

## What Keybox is

Applications such as browsers, mail clients, chat apps and Git tools store
passwords and tokens in your desktop **keyring**. Keybox lets you see what is
stored there, look at a secret when you need it, and tidy things up.

Keybox does not store anything itself. It talks to the **keyring service**
running in your session (oo7, GNOME Keyring, KWallet, KeePassXC or another
provider of the freedesktop Secret Service) and shows what that service
exposes. Any change you make in Keybox is made in the keyring service, where
all your applications see it.

A few words used throughout:

- **Keyring**: a named collection of entries, protected by a password. Most
  systems have a *Login* keyring, unlocked when you log in, and a temporary
  *session* keyring that is emptied when you log out.
- **Entry**: one stored item. It has a **label** (the name you see), a
  **secret** (the password or token) and **attributes**.
- **Attributes**: name/value pairs, such as `service = github.com` and
  `username = alice`. Applications find their entries by attributes, not by
  label.
- **Default keyring**: where applications store new entries unless they pick
  another keyring.

## Before you start

Keybox needs a keyring service running in your session. On COSMIC this is
usually oo7 (`oo7-daemon`); on other desktops it may be GNOME Keyring or
KWallet. Keybox never installs, starts or picks a provider for you. If none
is running, it says so (see [Connection problems](#connection-problems)).

Tested providers: oo7 0.6 and GNOME Keyring 51.1. Others should work for
browsing; changing data with an untested provider shows a warning first.

## The window

The header shows the connection status, for example *Connected to oo7*, and
the menus:

| Menu | Items |
| --- | --- |
| File | Refresh, Search, Quit |
| Keyring | Unlock keyring, Lock keyring…, New keyring…, Rename keyring…, Delete keyring… |
| Entry | Reveal or hide secret, Copy secret, New entry…, Edit entry…, Delete entry… |
| View | Settings…, About Keybox |

Items that do not apply right now (for example *Edit entry…* with no entry
selected, or *Rename keyring…* on a locked keyring) are greyed out.

The window has three parts:

1. **Sidebar**: your keyrings. Each shows whether it is the default keyring
   and whether it is locked or unlocked.
2. **Entry list**: the entries of the selected keyring, sorted by label.
3. **Details**: the selected entry, with its secret hidden.

When the window is narrower than about 960 pixels, the entry list and the
details share one pane: selecting an entry opens its details, and **Back**
(or Escape) returns to the list.

## Browsing and searching

Pick a keyring in the sidebar, then an entry in the list. Browsing only reads
labels, attributes and dates. **No secret is read until you choose Reveal or
Copy.**

What you may see in the entry list:

- **Untitled entry**: the entry has no label.
- **Locked entry**: the keyring is locked, and the service does not disclose
  the entry's label until it is unlocked (oo7 does this; GNOME Keyring shows
  labels even while locked).
- *This keyring is locked*: unlock it to see its entries.
- *The keyring service exposes no entries in this keyring*: the keyring is
  empty, as far as the service shows. A provider may keep data it does not
  expose.

**Search** (Ctrl+F, or File › Search) filters entries in all keyrings by
label and attribute names and values. It searches what Keybox has already
loaded, never secrets. The result line tells you if something could not be
searched, for example entries in locked keyrings, or keyrings that failed or
are still loading. Escape closes the search.

**Refresh** (Ctrl+R or F5) reloads keyrings and entries. You rarely need it:
Keybox follows changes made by other applications as they happen.

## Entry details

The details pane shows:

| Field | Meaning |
| --- | --- |
| Keyring | The keyring the entry is in |
| Status | Locked or unlocked |
| Created, Modified | Dates reported by the keyring service |
| Content type | What the service says the secret is, e.g. `text/plain` (shown once revealed) |
| Attributes | All name/value pairs. *Not disclosed while locked* if the service hides them |
| Technical details | The D-Bus object paths of the entry and its keyring, useful for bug reports |

Very long labels are shortened in the list; the details say how long the
full label is.

## Revealing and copying secrets

The **Secret** section starts hidden: *Nothing is retrieved until you choose
Reveal or Copy.*

**Reveal** (Ctrl+Shift+E) fetches the secret and shows it, with a countdown:
*Hides in 30 s*. **Hide** hides it at once. A revealed secret is also hidden,
and removed from memory, as soon as:

- the countdown runs out (change the time in [Settings](#settings)),
- you select another entry or keyring,
- the Keybox window loses focus, for example when you switch windows,
- the entry or its keyring changes or is deleted, or the keyring is locked,
- the connection to the keyring service is lost.

**Copy** (Ctrl+Shift+C) puts the secret on the clipboard without showing it.
After 30 seconds Keybox clears the clipboard, **but only if it still holds
exactly that secret**: if you copied something else since, it is left alone.
Keybox can only check the clipboard while its window is focused. If you have
switched away, the check happens when you come back to Keybox.

Special cases:

- **Empty secret**: Keybox says *This secret is empty* rather than showing
  nothing.
- **Line breaks or other control characters**: the secret is shown with
  escapes (such as `\n`) so you can see them. Copying uses the exact stored
  text, and Keybox asks first (*Copy text with hidden characters?*).
- **Binary content**: secrets that are not text are not shown or copied in
  this release; Keybox shows their size instead.

> **Clipboard managers** (such as the COSMIC clipboard manager applet) keep
> their own history of everything you copy, possibly on disk. Keybox cannot
> remove secrets from it. Prefer **Reveal** over **Copy**, or delete the entry
> from the clipboard manager's history after pasting.

## Locking and unlocking keyrings

**Lock keyring…** (Keyring menu) locks the selected keyring for every
application, not just for Keybox. Keybox asks for confirmation first, because
applications that need the keyring cannot read their passwords until it is
unlocked again.

**Unlock keyring** asks the keyring service to unlock it. The service then
asks for the keyring's password, normally through the desktop's password
prompt. While Keybox waits, it shows *Waiting for the keyring service to ask
for the password…* and a **Cancel** button. If no prompt appears, see the
hint shown in the window and the next section.

After a wrong password the prompt asks again. If you cancel the prompt, or no
prompt could be shown, Keybox says *Unlocking was cancelled, or no password
prompt could be shown.* (Some services do not report which of the two
happened.)

At login the Login keyring is usually unlocked for you, so you will mostly
need this for keyrings you locked yourself, or keyrings with their own
password.

## The built-in password prompt

Keyring services do not draw password dialogs themselves; they ask the
desktop's *prompter*. COSMIC has none yet, so on COSMIC an unlock request
would otherwise wait forever.

Keybox therefore brings a stand-in. When you unlock or create a keyring in
Keybox and no other prompter is available, Keybox shows its own dialog:

- **Unlock “name”**: enter the keyring's password and choose **Unlock**.
  If the password is wrong, the dialog appears again with *The password was
  incorrect. Try again.*
- **Password for “name”** (new keyring): enter the new password twice and
  choose **Create**. *The passwords do not match* is shown until both fields
  agree.

The eye button shows or hides what you type. **Cancel** or Escape cancels the
request.

What it does and does not do:

- It answers **only Keybox's own requests**. Other applications on COSMIC
  still get no prompt; unlock the keyring in Keybox first, and they can then
  use it.
- It is used only when your system has no other prompter installed or
  running. If one exists, Keybox leaves the job to it. Note that `gcr`
  (pulled in by `gnome-keyring` and `seahorse` on Arch) installs
  `gcr-prompter`, which counts even if it does not appear on COSMIC. If
  unlocking waits with no dialog, check whether `gcr` is installed.
- The password goes encrypted from Keybox to the keyring service and is not
  kept.
- It is tested with oo7. It is temporary and will be removed once COSMIC
  provides a prompter. Builds made with `--no-default-features` leave it out.

## Creating and editing entries

Entries can only be created and changed in an **unlocked** keyring. With an
untested keyring service, Keybox warns: *Changing stored data has not been
tested with this keyring service.*

### New entry

Choose **New entry** (Ctrl+N, or Entry › New entry…). The form has:

- **Label**: the name shown in the entry list. Required.
- **Secret**: the password or token, entered on one line. It is sent to the
  keyring service encrypted and never saved by Keybox. An empty secret is
  allowed, but Keybox points it out.
- **Attributes**: name/value pairs. Use **Add attribute** and **Remove
  attribute**. Every attribute needs a name, and each name can appear only
  once.

Choose **Create**.

### Edit

Select an entry and choose **Edit** (F2, or Entry › Edit entry…). You can
change the label and the attributes. The current secret is **not** loaded
into the form; it is kept unless you tick **Replace the secret** and enter a
new one.

Be careful with attributes: applications find their entries by them, so
changing them can make an application lose track of its password.

Choose **Save**. If you press Escape on a form you have not changed, it
closes.

### Checks before saving

- **Duplicate attributes**: if other entries in the keyring already have all
  of these attributes, Keybox tells you, because an application looking them
  up may find either one. Keybox never replaces an existing entry. Choose
  **Edit the existing entry** to change that one instead, or **Save anyway**
  to keep both.
- **Changed by another application**: Keybox reads the entry again just
  before saving. If another application changed it after you started
  editing, choose **Save and overwrite** or **Discard my changes and
  reload**. Keybox cannot stop a change made at the very same moment.
- **Deleted elsewhere**: *This entry no longer exists*; nothing is saved.
- **Partly saved**: label, attributes and secret are saved one after the
  other. If one step fails, Keybox says which parts were saved and which not.

## Deleting entries

Select an entry and choose **Delete** (the Delete key, or Entry › Delete
entry…). The confirmation names the entry and its keyring. Deleting is
permanent, and applications that use the entry will no longer find it.

## Managing keyrings

All in the **Keyring** menu.

- **New keyring…**: enter a name and choose **Create keyring**. The keyring
  service then asks for the new keyring's password (through the built-in
  prompt on COSMIC). The new keyring starts unlocked.
- **Rename keyring…**: the keyring must be unlocked.
- **Delete keyring…**: only for keyrings that are unlocked, fully loaded and
  **empty**. The default keyring and the session keyring can never be deleted
  from Keybox. Keybox checks once more that the keyring is empty right before
  deleting it.

Changing which keyring is the default is not offered in this release: oo7
0.6 does not do it reliably.

### Waiting for the keyring service

Every change shows *Waiting for the keyring service…* with **Cancel**, in
case the service needs a prompt that never appears. If you cancel, Keybox
stops waiting, but the service may still complete the change, so check the
result. If the service's own prompt is cancelled, nothing was changed.

## Settings

Open with **View › Settings…** or Ctrl+,.

- **Hide revealed secrets after**: 10, 30 (default), 60 or 120 seconds.
- **Clipboard**: explains how and when copied secrets are cleared (always
  after 30 seconds, see
  [Revealing and copying secrets](#revealing-and-copying-secrets)).

The reveal time is the only thing Keybox saves.

## Keyboard shortcuts

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

## Connection problems

If Keybox cannot talk to a keyring service, it shows a full-window message:

| Message | What to do |
| --- | --- |
| No keyring service is available | Install and enable a provider such as oo7 or GNOME Keyring, then **Try again** |
| The keyring service could not be started | A provider is configured but failed to start. Check its installation, then **Try again** |
| Access to the keyring service was denied | The service refused the connection. Check the provider's own settings or logs |
| The keyring service went away | The service stopped or was replaced. Choose **Reconnect**; entries from the old connection are not reused |
| Could not connect to the keyring service | Shows the reason; **Try again** |

## Messages and what they mean

Error messages end with a short reason. The most common ones:

| Reason | Meaning |
| --- | --- |
| the keyring is locked | Unlock the keyring first |
| it no longer exists | Another application deleted it; the list updates by itself |
| it belongs to an earlier connection | The service restarted; refresh or reconnect |
| the prompt was dismissed | The password prompt was cancelled |
| the connection was lost | The keyring service stopped |
| the keyring service did not respond in time | Try again; the service may be busy or stuck |
| the secure session expired | Try the action again |
| the keyring service does not support encrypted transfer | Keybox refuses to transfer secrets unencrypted, so Reveal, Copy and saving secrets are unavailable with this service |
| the keyring service accepted the request but did not apply it | The service said yes but nothing changed |
| the keyring is not empty | Only empty keyrings can be deleted |
| not supported by the keyring service | This provider does not offer the action |

*The keyring service left “name” unlocked*: you asked to lock a keyring, but
the service kept it unlocked.

## Privacy and security

- Keybox reads a secret only when you choose Reveal or Copy, and forgets it
  when it is hidden.
- Secrets always travel over an encrypted session between Keybox and the
  keyring service. Keybox never falls back to unencrypted transfer.
- Memory that held a secret or a typed password is wiped after use. One
  exception: the text fields of the toolkit keep their own copy, which Keybox
  cannot wipe.
- Keybox writes no logs, makes no network requests, and saves nothing except
  the reveal time.
- Keybox does not protect against a compromised user session: any program
  running as you can ask the keyring service for secrets, just like Keybox.

## Known limits

- **Screen readers** have not been verified.
- **Session lock**: secrets are hidden when the window loses focus, but
  Keybox does not react to the screen locking as such.
- **Other applications on COSMIC** cannot unlock keyrings themselves until
  COSMIC has a prompter; unlock in Keybox first.
- **Password prompts from the desktop** are not attached to the Keybox
  window yet.
- **KWallet and KeePassXC** are untested. KeePassXC only exposes the entries
  its integration is set up to share.
- **Binary secrets** cannot be viewed or copied.
- **Default keyring** cannot be changed from Keybox.

Details of what has been tested: [`providers.md`](providers.md) and
[`qualification.md`](qualification.md).
