// SPDX-License-Identifier: GPL-3.0-only

//! Minimal `org.freedesktop.Secret.*` proxies.
//!
//! Only read operations, lock and unlock are declared. Properties are read
//! through `org.freedesktop.DBus.Properties.GetAll` in `client.rs`.

use std::collections::HashMap;

use serde::Deserialize;
use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Type, Value};

pub const SERVICE_NAME: &str = "org.freedesktop.secrets";
pub const SERVICE_PATH: &str = "/org/freedesktop/secrets";
pub const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
pub const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
pub const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";

/// The `(oayays)` secret structure. `content_type` is kept as a string:
/// providers send variants such as `text/plain; charset=utf8`.
#[derive(Deserialize, Type)]
pub struct RawSecret {
    pub session: OwnedObjectPath,
    pub parameters: Vec<u8>,
    pub value: Vec<u8>,
    pub content_type: String,
}

impl Drop for RawSecret {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        // Encrypted on the wire, but wipe it anyway.
        self.value.zeroize();
    }
}

#[proxy(interface = "org.freedesktop.Secret.Service", gen_blocking = false)]
pub trait Service {
    fn open_session(
        &self,
        algorithm: &str,
        input: &Value<'_>,
    ) -> zbus::Result<(OwnedValue, OwnedObjectPath)>;

    fn unlock(
        &self,
        objects: &[ObjectPath<'_>],
    ) -> zbus::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)>;

    fn lock(
        &self,
        objects: &[ObjectPath<'_>],
    ) -> zbus::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)>;

    fn read_alias(&self, name: &str) -> zbus::Result<OwnedObjectPath>;

    fn get_secrets(
        &self,
        items: &[ObjectPath<'_>],
        session: &ObjectPath<'_>,
    ) -> zbus::Result<HashMap<OwnedObjectPath, RawSecret>>;
}

#[proxy(interface = "org.freedesktop.Secret.Item", gen_blocking = false)]
pub trait Item {
    fn get_secret(&self, session: &ObjectPath<'_>) -> zbus::Result<RawSecret>;
}

#[proxy(interface = "org.freedesktop.Secret.Prompt", gen_blocking = false)]
pub trait Prompt {
    fn prompt(&self, window_id: &str) -> zbus::Result<()>;

    fn dismiss(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn completed(&self, dismissed: bool, result: Value<'_>) -> zbus::Result<()>;
}

#[proxy(interface = "org.freedesktop.Secret.Session", gen_blocking = false)]
pub trait Session {
    fn close(&self) -> zbus::Result<()>;
}
