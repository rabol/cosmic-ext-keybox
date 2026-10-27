// SPDX-License-Identifier: GPL-3.0-only

//! The Secret Service client.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use futures_util::stream::{self, Stream, StreamExt};
use zbus::fdo::{DBusProxy, PropertiesProxy};
use zbus::message::Type as MessageType;
use zbus::names::{BusName, InterfaceName, OwnedUniqueName};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{MatchRule, Message, MessageStream};

use super::crypto::{self, KeyPair, SessionKey};
use super::proxy::{
    COLLECTION_INTERFACE, CollectionProxy, ITEM_INTERFACE, ItemProxy, OutgoingSecret, PromptProxy,
    SERVICE_INTERFACE, SERVICE_NAME, SERVICE_PATH, ServiceProxy,
};
use super::{CollectionInfo, Error, ItemInfo, ObjectId, Provider, RetrievedSecret, ServiceEvent};
use crate::secret::{SecretText, SecretValue};

const METHOD_TIMEOUT: Duration = Duration::from_secs(30);
/// Content type for text secrets Keybox stores.
const TEXT_CONTENT_TYPE: &str = "text/plain";
/// How many metadata requests may be in flight at once.
const METADATA_CONCURRENCY: usize = 16;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Which bus to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bus {
    /// The user's session bus.
    Session,
    /// An explicit bus address. Used by tests with an isolated bus.
    Address(String),
}

/// A connection to the active `org.freedesktop.secrets` owner. Cheap to clone.
///
/// Browsing works without transfer encryption; retrieving a secret requires
/// it and never falls back to plaintext.
///
/// All calls go to the owner's unique bus name, so a replacement service
/// can never receive requests meant for the previous one.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    conn: zbus::Connection,
    generation: u64,
    owner: OwnedUniqueName,
    /// The encrypted transfer session, if the service supports one.
    session: Option<EncryptedSession>,
    provider: Option<Provider>,
}

struct EncryptedSession {
    path: OwnedObjectPath,
    key: SessionKey,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("generation", &self.inner.generation)
            .field("provider", &self.inner.provider)
            .finish_non_exhaustive()
    }
}

/// Outcome of a lock or unlock request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The service finished without asking the user.
    Done,
    /// The user completed the provider's prompt.
    Completed,
}

impl Client {
    /// Connects to the active Secret Service and tries to open an encrypted
    /// transfer session.
    ///
    /// Uses normal D-Bus activation if the service is not running. If the
    /// service does not support encryption, the connection still succeeds
    /// for browsing, and [`Client::secret`] fails with
    /// [`Error::EncryptionUnavailable`]. A plaintext session is never opened.
    pub async fn connect(bus: &Bus) -> Result<Self, Error> {
        let builder = match bus {
            Bus::Session => zbus::connection::Builder::session()?,
            Bus::Address(address) => zbus::connection::Builder::address(address.as_str())?,
        };
        let conn = builder.method_timeout(METHOD_TIMEOUT).build().await?;

        let dbus = DBusProxy::new(&conn).await?;
        let name = BusName::try_from(SERVICE_NAME).map_err(|_| Error::Protocol)?;
        // Ask for activation only when nobody owns the name: some bus
        // implementations reject StartServiceByName without a service file
        // even while the name is owned.
        let owner = match dbus.get_name_owner(name.clone()).await {
            Ok(owner) => owner,
            Err(zbus::fdo::Error::NameHasNoOwner(_)) => {
                dbus.start_service_by_name(
                    SERVICE_NAME.try_into().map_err(|_| Error::Protocol)?,
                    0,
                )
                .await?;
                dbus.get_name_owner(name).await?
            }
            Err(err) => return Err(err.into()),
        };

        let service = service_proxy(&conn, &owner).await?;
        let session = open_encrypted_session(&service).await?;
        let provider = detect_provider(&dbus, &owner).await;

        Ok(Self {
            inner: Arc::new(Inner {
                conn,
                generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
                owner,
                session,
                provider,
            }),
        })
    }

    /// Identifies this connection. Bumped on every reconnect.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation
    }

    /// Whether secrets can be retrieved (an encrypted session exists).
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.inner.session.is_some()
    }

    /// The provider, when recognised from the owning process.
    #[must_use]
    pub fn provider(&self) -> Option<Provider> {
        self.inner.provider
    }

    /// Lists collections with their metadata. Fetches no secrets.
    pub async fn collections(&self) -> Result<Vec<CollectionInfo>, Error> {
        let mut props = self.get_all(SERVICE_PATH, SERVICE_INTERFACE).await?;
        let paths: Vec<OwnedObjectPath> = take(&mut props, "Collections").ok_or(Error::Protocol)?;

        let service = self.service().await?;
        let default = match service.read_alias("default").await {
            Ok(path) if path.as_str() != "/" => Ok(Some(path)),
            Ok(_) => Ok(None),
            Err(_) => Err(()),
        };
        let session = service.read_alias("session").await.ok();

        let infos = stream::iter(paths)
            .map(|path| {
                let default = &default;
                let is_session = session.as_ref() == Some(&path)
                    || path.as_str() == "/org/freedesktop/secrets/collection/session";
                async move {
                    let id = self.id(path);
                    let is_default = default
                        .as_ref()
                        .ok()
                        .map(|d| d.as_ref().is_some_and(|d| d == &id.path));
                    match self.get_all(id.path.as_str(), COLLECTION_INTERFACE).await {
                        Ok(mut p) => CollectionInfo {
                            label: take(&mut p, "Label"),
                            locked: take(&mut p, "Locked"),
                            created: take(&mut p, "Created").and_then(timestamp),
                            modified: take(&mut p, "Modified").and_then(timestamp),
                            is_default,
                            is_session,
                            error: None,
                            id,
                        },
                        Err(error) => CollectionInfo {
                            id,
                            label: None,
                            locked: None,
                            created: None,
                            modified: None,
                            is_default,
                            is_session,
                            error: Some(error),
                        },
                    }
                }
            })
            .buffered(METADATA_CONCURRENCY)
            .collect()
            .await;
        Ok(infos)
    }

    /// Lists a collection's entries with their metadata. Fetches no secrets.
    pub async fn items(&self, collection: &ObjectId) -> Result<Vec<ItemInfo>, Error> {
        self.check(collection)?;
        let paths = self.item_paths(collection).await?;
        let infos = stream::iter(paths)
            .map(|path| self.item_info(collection.clone(), self.id(path)))
            .buffered(METADATA_CONCURRENCY)
            .collect()
            .await;
        Ok(infos)
    }

    /// Re-reads one entry's metadata, for example after `ItemChanged`.
    pub async fn item(&self, collection: &ObjectId, item: &ObjectId) -> Result<ItemInfo, Error> {
        self.check(collection)?;
        self.check(item)?;
        let info = self.item_info(collection.clone(), item.clone()).await;
        match info.error {
            Some(Error::NoSuchObject) => Err(Error::NoSuchObject),
            // Some providers (GNOME Keyring) answer a deleted path with a
            // generic error. Check the collection's entry list to tell.
            Some(_) if !self.collection_contains(collection, item).await? => {
                Err(Error::NoSuchObject)
            }
            _ => Ok(info),
        }
    }

    async fn collection_contains(
        &self,
        collection: &ObjectId,
        item: &ObjectId,
    ) -> Result<bool, Error> {
        Ok(self.item_paths(collection).await?.contains(&item.path))
    }

    async fn item_paths(&self, collection: &ObjectId) -> Result<Vec<OwnedObjectPath>, Error> {
        let mut props = self
            .get_all(collection.path.as_str(), COLLECTION_INTERFACE)
            .await?;
        take(&mut props, "Items").ok_or(Error::Protocol)
    }

    async fn item_info(&self, collection: ObjectId, id: ObjectId) -> ItemInfo {
        match self.get_all(id.path.as_str(), ITEM_INTERFACE).await {
            Ok(mut p) => ItemInfo {
                label: take(&mut p, "Label"),
                attributes: take::<HashMap<String, String>>(&mut p, "Attributes")
                    .map(|a| a.into_iter().collect::<BTreeMap<_, _>>()),
                locked: take(&mut p, "Locked"),
                created: take(&mut p, "Created").and_then(timestamp),
                modified: take(&mut p, "Modified").and_then(timestamp),
                error: None,
                id,
                collection,
            },
            Err(error) => ItemInfo {
                id,
                collection,
                label: None,
                attributes: None,
                locked: None,
                created: None,
                modified: None,
                error: Some(error),
            },
        }
    }

    /// Retrieves one entry's secret over the encrypted session.
    ///
    /// Returns [`Error::Locked`] if the entry must be unlocked first. Never
    /// returns an empty secret in place of an error.
    pub async fn secret(&self, item: &ObjectId) -> Result<RetrievedSecret, Error> {
        self.check(item)?;
        let session = self
            .inner
            .session
            .as_ref()
            .ok_or(Error::EncryptionUnavailable)?;
        let raw = self
            .item_proxy(item)
            .await?
            .get_secret(&session.path.as_ref())
            .await?;
        if raw.session != session.path {
            return Err(Error::Protocol);
        }
        // An encrypted session always carries an IV. Empty parameters would
        // mean plaintext, which `decrypt` rejects.
        let bytes = session.key.decrypt(&raw.parameters, &raw.value)?;
        Ok(RetrievedSecret {
            value: SecretValue::from_bytes(bytes),
            content_type: raw.content_type.clone(),
        })
    }

    /// Asks the service to unlock objects. The provider shows its own prompt
    /// if needed. Dropping the future dismisses a pending prompt.
    pub async fn unlock(
        &self,
        objects: &[ObjectId],
        window_id: Option<&str>,
    ) -> Result<PromptOutcome, Error> {
        let paths = self.paths(objects)?;
        let (_, prompt) = self.service().await?.unlock(&paths).await?;
        Ok(self.run_prompt(prompt, window_id).await?.0)
    }

    /// Asks the service to lock objects. This may affect other applications.
    /// Re-read the `locked` state afterwards: a provider may ignore it.
    pub async fn lock(
        &self,
        objects: &[ObjectId],
        window_id: Option<&str>,
    ) -> Result<PromptOutcome, Error> {
        let paths = self.paths(objects)?;
        let (_, prompt) = self.service().await?.lock(&paths).await?;
        Ok(self.run_prompt(prompt, window_id).await?.0)
    }

    // --- changes to stored data ---

    /// Finds entries in a collection whose attributes include all of
    /// `attributes`: the entries an application looking them up would find.
    pub async fn find_items(
        &self,
        collection: &ObjectId,
        attributes: &BTreeMap<String, String>,
    ) -> Result<Vec<ObjectId>, Error> {
        self.check(collection)?;
        let query: HashMap<&str, &str> = attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let paths = self
            .collection_proxy(collection)
            .await?
            .search_items(query)
            .await?;
        Ok(paths.into_iter().map(|p| self.id(p)).collect())
    }

    /// Creates a text entry. Never replaces an existing entry, even one
    /// with the same attributes. The provider may prompt (for example if
    /// the keyring is locked).
    pub async fn create_item(
        &self,
        collection: &ObjectId,
        label: &str,
        attributes: &BTreeMap<String, String>,
        secret: &SecretText,
        window_id: Option<&str>,
    ) -> Result<ObjectId, Error> {
        self.check(collection)?;
        let secret = self.outgoing(secret)?;
        let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
        properties.insert("org.freedesktop.Secret.Item.Label", Value::from(label));
        let attributes: HashMap<&str, &str> = attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        properties.insert(
            "org.freedesktop.Secret.Item.Attributes",
            Value::from(attributes),
        );
        let (item, prompt) = self
            .collection_proxy(collection)
            .await?
            .create_item(properties, &secret, false)
            .await?;
        if item.as_str() != "/" {
            return Ok(self.id(item));
        }
        let (_, result) = self.run_prompt(prompt, window_id).await?;
        let item = result
            .and_then(|v| OwnedObjectPath::try_from(v).ok())
            .filter(|p| p.as_str() != "/")
            .ok_or(Error::Protocol)?;
        Ok(self.id(item))
    }

    /// Changes an entry's label.
    pub async fn set_item_label(&self, item: &ObjectId, label: &str) -> Result<(), Error> {
        self.set_property(item, ITEM_INTERFACE, "Label", Value::from(label))
            .await
    }

    /// Replaces an entry's attributes. Applications find their entries by
    /// attributes, so this can make an application lose track of one.
    pub async fn set_item_attributes(
        &self,
        item: &ObjectId,
        attributes: &BTreeMap<String, String>,
    ) -> Result<(), Error> {
        let attributes: HashMap<&str, &str> = attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.set_property(item, ITEM_INTERFACE, "Attributes", Value::from(attributes))
            .await
    }

    /// Replaces an entry's secret with text, over the encrypted session.
    pub async fn set_item_secret(&self, item: &ObjectId, secret: &SecretText) -> Result<(), Error> {
        self.check(item)?;
        let secret = self.outgoing(secret)?;
        self.item_proxy(item).await?.set_secret(&secret).await?;
        Ok(())
    }

    /// Deletes an entry. The provider may prompt.
    pub async fn delete_item(
        &self,
        item: &ObjectId,
        window_id: Option<&str>,
    ) -> Result<PromptOutcome, Error> {
        self.check(item)?;
        let prompt = self.item_proxy(item).await?.delete().await?;
        Ok(self.run_prompt(prompt, window_id).await?.0)
    }

    /// Creates a collection. Providers normally prompt for its password.
    pub async fn create_collection(
        &self,
        label: &str,
        window_id: Option<&str>,
    ) -> Result<ObjectId, Error> {
        let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
        properties.insert(
            "org.freedesktop.Secret.Collection.Label",
            Value::from(label),
        );
        let (collection, prompt) = self
            .service()
            .await?
            .create_collection(properties, "")
            .await?;
        if collection.as_str() != "/" {
            return Ok(self.id(collection));
        }
        let (_, result) = self.run_prompt(prompt, window_id).await?;
        let collection = result
            .and_then(|v| OwnedObjectPath::try_from(v).ok())
            .filter(|p| p.as_str() != "/")
            .ok_or(Error::Protocol)?;
        Ok(self.id(collection))
    }

    /// Changes a collection's label.
    pub async fn set_collection_label(
        &self,
        collection: &ObjectId,
        label: &str,
    ) -> Result<(), Error> {
        self.set_property(
            collection,
            COLLECTION_INTERFACE,
            "Label",
            Value::from(label),
        )
        .await
    }

    /// Points the `default` alias at a collection. Applications store new
    /// entries there unless they choose a collection themselves.
    ///
    /// Reads the alias back and returns [`Error::NotApplied`] if the
    /// service accepted the call but the alias did not move (oo7 0.6.0
    /// does this when moving the default back to an earlier keyring).
    pub async fn set_default_collection(&self, collection: &ObjectId) -> Result<(), Error> {
        self.check(collection)?;
        let service = self.service().await?;
        service
            .set_alias("default", &collection.path.as_ref())
            .await?;
        if service.read_alias("default").await? == collection.path {
            Ok(())
        } else {
            Err(Error::NotApplied)
        }
    }

    /// Deletes a collection, but only if it currently has no entries.
    ///
    /// The emptiness check and the deletion are separate calls; an entry
    /// added in between by another application would be deleted with it.
    pub async fn delete_empty_collection(
        &self,
        collection: &ObjectId,
        window_id: Option<&str>,
    ) -> Result<PromptOutcome, Error> {
        self.check(collection)?;
        if !self.item_paths(collection).await?.is_empty() {
            return Err(Error::NotEmpty);
        }
        let prompt = self.collection_proxy(collection).await?.delete().await?;
        Ok(self.run_prompt(prompt, window_id).await?.0)
    }

    /// Runs a prompt if the service returned one. Returns the prompt's
    /// result value (for example a created object's path) when completed.
    async fn run_prompt(
        &self,
        prompt: OwnedObjectPath,
        window_id: Option<&str>,
    ) -> Result<(PromptOutcome, Option<OwnedValue>), Error> {
        if prompt.as_str() == "/" {
            return Ok((PromptOutcome::Done, None));
        }
        let proxy = PromptProxy::builder(&self.inner.conn)
            .destination(self.inner.owner.clone())?
            .path(prompt)?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        // Subscribe before calling Prompt so the signal cannot be missed.
        let mut completed = proxy.receive_completed().await?;
        let mut guard = DismissOnDrop(Some(proxy.clone()));
        proxy.prompt(window_id.unwrap_or("")).await?;
        let signal = completed.next().await.ok_or(Error::Disconnected)?;
        guard.0 = None;
        let args = signal.args()?;
        if args.dismissed {
            Err(Error::PromptDismissed)
        } else {
            let result = args.result.try_to_owned().ok();
            Ok((PromptOutcome::Completed, result))
        }
    }

    /// Change notifications from this service, plus owner changes.
    ///
    /// The stream ends when the bus connection is lost; treat that as a
    /// disconnect.
    pub async fn events(
        &self,
    ) -> Result<impl Stream<Item = ServiceEvent> + Send + 'static + use<>, Error> {
        let conn = &self.inner.conn;
        let owner = self.inner.owner.clone();
        let from_owner = |interface: &'static str| -> Result<MatchRule<'static>, Error> {
            Ok(MatchRule::builder()
                .msg_type(MessageType::Signal)
                .sender(owner.clone())?
                .interface(interface)?
                .build())
        };
        let rules = [
            from_owner(SERVICE_INTERFACE)?,
            from_owner(COLLECTION_INTERFACE)?,
            from_owner("org.freedesktop.DBus.Properties")?,
            MatchRule::builder()
                .msg_type(MessageType::Signal)
                .sender("org.freedesktop.DBus")?
                .interface("org.freedesktop.DBus")?
                .member("NameOwnerChanged")?
                .add_arg(SERVICE_NAME)?
                .build(),
        ];
        let mut streams = Vec::with_capacity(rules.len());
        for rule in rules {
            streams.push(MessageStream::for_match_rule(rule, conn, Some(256)).await?);
        }

        let generation = self.inner.generation;
        let owner = self.inner.owner.clone();
        Ok(stream::select_all(streams).filter_map(move |msg| {
            let event = msg.ok().and_then(|msg| to_event(&msg, generation, &owner));
            async move { event }
        }))
    }

    // --- helpers ---

    fn id(&self, path: OwnedObjectPath) -> ObjectId {
        ObjectId {
            generation: self.inner.generation,
            path,
        }
    }

    fn check(&self, id: &ObjectId) -> Result<(), Error> {
        if id.generation == self.inner.generation {
            Ok(())
        } else {
            Err(Error::Stale)
        }
    }

    fn paths<'a>(&self, ids: &'a [ObjectId]) -> Result<Vec<ObjectPath<'a>>, Error> {
        ids.iter()
            .map(|id| self.check(id).map(|()| id.path.as_ref()))
            .collect()
    }

    /// Encrypts a text secret for this session. Fails rather than sending
    /// plaintext when no encrypted session exists.
    fn outgoing(&self, secret: &SecretText) -> Result<OutgoingSecret, Error> {
        let session = self
            .inner
            .session
            .as_ref()
            .ok_or(Error::EncryptionUnavailable)?;
        let (parameters, value) = session.key.encrypt(secret.expose().as_bytes())?;
        Ok(OutgoingSecret {
            session: session.path.clone(),
            parameters,
            value,
            content_type: TEXT_CONTENT_TYPE.to_owned(),
        })
    }

    async fn item_proxy(&self, item: &ObjectId) -> Result<ItemProxy<'static>, Error> {
        Ok(ItemProxy::builder(&self.inner.conn)
            .destination(self.inner.owner.clone())?
            .path(item.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?)
    }

    async fn collection_proxy(
        &self,
        collection: &ObjectId,
    ) -> Result<CollectionProxy<'static>, Error> {
        Ok(CollectionProxy::builder(&self.inner.conn)
            .destination(self.inner.owner.clone())?
            .path(collection.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?)
    }

    async fn set_property(
        &self,
        object: &ObjectId,
        interface: &'static str,
        name: &str,
        value: Value<'_>,
    ) -> Result<(), Error> {
        self.check(object)?;
        let proxy = PropertiesProxy::builder(&self.inner.conn)
            .destination(self.inner.owner.clone())?
            .path(object.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let interface = InterfaceName::from_static_str(interface).map_err(|_| Error::Protocol)?;
        proxy.set(interface, name, value).await?;
        Ok(())
    }

    async fn service(&self) -> Result<ServiceProxy<'static>, Error> {
        service_proxy(&self.inner.conn, &self.inner.owner).await
    }

    async fn get_all(
        &self,
        path: &str,
        interface: &'static str,
    ) -> Result<HashMap<String, OwnedValue>, Error> {
        let proxy = PropertiesProxy::builder(&self.inner.conn)
            .destination(self.inner.owner.clone())?
            .path(path.to_owned())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let interface = InterfaceName::from_static_str(interface).map_err(|_| Error::Protocol)?;
        Ok(proxy.get_all(interface).await?)
    }
}

async fn service_proxy(
    conn: &zbus::Connection,
    owner: &OwnedUniqueName,
) -> Result<ServiceProxy<'static>, Error> {
    Ok(ServiceProxy::builder(conn)
        .destination(owner.clone())?
        .path(SERVICE_PATH)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?)
}

/// Opens a `dh-ietf1024-sha256-aes128-cbc-pkcs7` session. Returns `None` if
/// the service does not support that algorithm.
async fn open_encrypted_session(
    service: &ServiceProxy<'_>,
) -> Result<Option<EncryptedSession>, Error> {
    let pair = KeyPair::generate()?;
    let input = Value::from(pair.public_key().to_vec());
    match service.open_session(crypto::ALGORITHM, &input).await {
        Ok((output, path)) => {
            let service_public: Vec<u8> = output.try_into().map_err(|_| Error::Protocol)?;
            let key = pair.derive_key(&service_public)?;
            Ok(Some(EncryptedSession { path, key }))
        }
        Err(err) => match Error::from(err) {
            Error::Unsupported => Ok(None),
            other => Err(other),
        },
    }
}

/// Recognises the provider from the owning process name. Display only.
async fn detect_provider(dbus: &DBusProxy<'_>, owner: &OwnedUniqueName) -> Option<Provider> {
    let pid = dbus
        .get_connection_unix_process_id(BusName::Unique(owner.as_ref()))
        .await
        .ok()?;
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Provider::from_process_name(&comm)
}

/// Removes a property and converts it, or `None` if missing or mistyped.
fn take<T>(props: &mut HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    props.remove(key).and_then(|v| T::try_from(v).ok())
}

/// Secret Service timestamps are seconds since the epoch; 0 means unknown.
fn timestamp(secs: u64) -> Option<SystemTime> {
    (secs != 0).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

fn to_event(msg: &Message, generation: u64, owner: &OwnedUniqueName) -> Option<ServiceEvent> {
    let header = msg.header();
    let member = header.member()?.as_str().to_owned();
    let interface = header.interface()?.as_str().to_owned();
    let path = OwnedObjectPath::from(header.path()?.to_owned());
    let id = |path: OwnedObjectPath| ObjectId { generation, path };
    let body = msg.body();

    match (interface.as_str(), member.as_str()) {
        ("org.freedesktop.DBus", "NameOwnerChanged") => {
            let (_name, _old, new): (String, String, String) = body.deserialize().ok()?;
            (new != owner.as_str()).then_some(ServiceEvent::OwnerChanged)
        }
        (SERVICE_INTERFACE, member) => {
            let collection = id(body.deserialize::<OwnedObjectPath>().ok()?);
            match member {
                "CollectionCreated" => Some(ServiceEvent::CollectionCreated(collection)),
                "CollectionDeleted" => Some(ServiceEvent::CollectionDeleted(collection)),
                "CollectionChanged" => Some(ServiceEvent::CollectionChanged(collection)),
                _ => None,
            }
        }
        (COLLECTION_INTERFACE, member) => {
            let collection = id(path);
            let item = id(body.deserialize::<OwnedObjectPath>().ok()?);
            match member {
                "ItemCreated" => Some(ServiceEvent::ItemCreated { collection, item }),
                "ItemChanged" => Some(ServiceEvent::ItemChanged { collection, item }),
                "ItemDeleted" => Some(ServiceEvent::ItemDeleted { collection, item }),
                _ => None,
            }
        }
        ("org.freedesktop.DBus.Properties", "PropertiesChanged") => {
            Some(ServiceEvent::PropertiesChanged(id(path)))
        }
        _ => None,
    }
}

/// Dismisses a pending prompt if the waiting future is dropped.
struct DismissOnDrop(Option<PromptProxy<'static>>);

impl Drop for DismissOnDrop {
    fn drop(&mut self) {
        if let Some(proxy) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = proxy.dismiss().await;
            });
        }
    }
}
