// SPDX-License-Identifier: GPL-3.0-only

//! Provider-neutral metadata types.
//!
//! Labels and attribute values are untrusted and may be sensitive, so the
//! `Debug` implementations print only paths, flags and counts.

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use zbus::zvariant::OwnedObjectPath;

use super::Error;

/// Identifies a service object: the connection generation plus the D-Bus
/// object path. Never identify objects by label.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ObjectId {
    pub(crate) generation: u64,
    pub(crate) path: OwnedObjectPath,
}

impl ObjectId {
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The D-Bus object path, for the technical details view.
    #[must_use]
    pub fn path(&self) -> &str {
        self.path.as_str()
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({}:{})", self.generation, self.path.as_str())
    }
}

/// A provider recognised from the service owner's process name. Only shown
/// when detected; otherwise the UI says "Secret Service".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Oo7,
    GnomeKeyring,
    KWallet,
    KeePassXc,
}

impl Provider {
    pub(crate) fn from_process_name(name: &str) -> Option<Self> {
        match name.trim() {
            "oo7-daemon" => Some(Self::Oo7),
            // /proc/<pid>/comm is truncated to 15 bytes.
            "gnome-keyring-d" | "gnome-keyring-daemon" => Some(Self::GnomeKeyring),
            "kwalletd5" | "kwalletd6" | "ksecretd" => Some(Self::KWallet),
            "keepassxc" => Some(Self::KeePassXc),
            _ => None,
        }
    }

    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Oo7 => "oo7",
            Self::GnomeKeyring => "GNOME Keyring",
            Self::KWallet => "KWallet",
            Self::KeePassXc => "KeePassXC",
        }
    }
}

/// A collection, shown as a "Keyring" in the UI.
///
/// Fields are `None` when the service did not supply them. When reading the
/// collection failed altogether, `error` says why.
#[derive(Clone)]
pub struct CollectionInfo {
    pub id: ObjectId,
    pub label: Option<String>,
    pub locked: Option<bool>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    /// Whether the `default` alias points here. `None` if the alias could not be read.
    pub is_default: Option<bool>,
    pub error: Option<Error>,
}

impl fmt::Debug for CollectionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollectionInfo")
            .field("id", &self.id)
            .field("has_label", &self.label.as_ref().map(|l| !l.is_empty()))
            .field("locked", &self.locked)
            .field("is_default", &self.is_default)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// An entry's metadata. Never contains the secret.
#[derive(Clone)]
pub struct ItemInfo {
    pub id: ObjectId,
    pub collection: ObjectId,
    pub label: Option<String>,
    /// `None` when the service did not disclose attributes (for example while locked).
    pub attributes: Option<BTreeMap<String, String>>,
    pub locked: Option<bool>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub error: Option<Error>,
}

impl fmt::Debug for ItemInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ItemInfo")
            .field("id", &self.id)
            .field("collection", &self.collection)
            .field("has_label", &self.label.as_ref().map(|l| !l.is_empty()))
            .field(
                "attribute_count",
                &self.attributes.as_ref().map(BTreeMap::len),
            )
            .field("locked", &self.locked)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// A retrieved secret plus its declared content type.
#[derive(Debug)]
pub struct RetrievedSecret {
    pub value: crate::secret::SecretValue,
    /// As declared by the service, e.g. `text/plain`. Informational only.
    pub content_type: String,
}

/// Something changed on the service. Refresh the affected metadata; never
/// fetch secrets in response to an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceEvent {
    CollectionCreated(ObjectId),
    CollectionDeleted(ObjectId),
    CollectionChanged(ObjectId),
    ItemCreated {
        collection: ObjectId,
        item: ObjectId,
    },
    ItemChanged {
        collection: ObjectId,
        item: ObjectId,
    },
    ItemDeleted {
        collection: ObjectId,
        item: ObjectId,
    },
    /// A property changed on some object (for example `Locked`).
    PropertiesChanged(ObjectId),
    /// The `org.freedesktop.secrets` owner went away or changed. Every
    /// `ObjectId` from this connection is now invalid; reconnect.
    OwnerChanged,
}
