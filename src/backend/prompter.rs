// SPDX-License-Identifier: GPL-3.0-only

//! A stand-in keyring prompter, for Keybox's own requests only.
//!
//! Secret Service providers do not draw unlock dialogs themselves: oo7 and
//! GNOME Keyring ask `org.gnome.keyring.SystemPrompter` (GNOME Shell or
//! `gcr-prompter`). COSMIC provides no prompter, so an unlock never completes.
//!
//! While one of Keybox's requests is pending, [`Prompter`] owns that name and
//! answers prompts whose caller window is Keybox's random token. Prompts for
//! other applications are left unanswered, exactly as they would be without
//! Keybox. It never takes the name when a real prompter exists (owned or
//! activatable), and releases it as soon as the request ends.
//!
//! This is temporary. Remove it (the `builtin-prompter` feature) once COSMIC
//! ships a prompter. See docs/providers.md.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Mutex;

use base64::prelude::{BASE64_STANDARD, Engine as _};
use tokio::sync::{mpsc, oneshot};
use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::names::{BusName, OwnedUniqueName};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroizing;

use super::crypto::{Group, KeyPair, SessionKey};
use super::{Bus, Client, Error, Provider};

const PROMPTER_NAME: &str = "org.gnome.keyring.SystemPrompter";
const PROMPTER_PATH: &str = "/org/gnome/keyring/Prompter";
const EXCHANGE_HEADER: &str = "[sx-aes-1]";

/// What the provider asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// The password of an existing keyring.
    Unlock,
    /// A password for a new keyring.
    NewPassword,
}

/// A password request for one of Keybox's own prompts. Answer it with
/// [`PasswordRequest::submit`]; dropping it cancels the prompt.
pub struct PasswordRequest {
    pub kind: PromptKind,
    /// The provider rejected the previous password.
    pub retry: bool,
    reply: oneshot::Sender<Option<Zeroizing<String>>>,
}

impl std::fmt::Debug for PasswordRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordRequest")
            .field("kind", &self.kind)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl PasswordRequest {
    /// Sends the password to the provider, encrypted for it.
    pub fn submit(self, password: Zeroizing<String>) {
        let _ = self.reply.send(Some(password));
    }

    /// Answers "Cancel"; the provider dismisses its prompt.
    pub fn cancel(self) {
        let _ = self.reply.send(None);
    }
}

/// Password requests from a running [`Prompter`].
pub struct Requests(mpsc::UnboundedReceiver<PasswordRequest>);

impl Requests {
    /// The next request. `None` once the prompter has stopped.
    pub async fn next(&mut self) -> Option<PasswordRequest> {
        self.0.recv().await
    }
}

/// Owns the prompter name while alive. Dropping it releases the name.
pub struct Prompter {
    conn: zbus::Connection,
    window_id: String,
}

impl std::fmt::Debug for Prompter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prompter")
    }
}

impl Prompter {
    /// Starts answering Keybox's prompts for `client`'s provider. Returns
    /// `None` when that is not needed or not possible: the provider does not
    /// use the GNOME prompter protocol, or a real prompter is available.
    pub async fn start(client: &Client) -> Result<Option<(Self, Requests)>, Error> {
        if !matches!(
            client.provider(),
            Some(Provider::Oo7 | Provider::GnomeKeyring)
        ) {
            return Ok(None);
        }
        let builder = match client.bus() {
            Bus::Session => zbus::connection::Builder::session()?,
            Bus::Address(address) => zbus::connection::Builder::address(address.as_str())?,
        };
        let token = random_token()?;
        // oo7 only forwards window identifiers it can parse, so the token
        // takes the form of a Wayland handle. Only this prompter reads it.
        let window_id = format!("wayland:keybox-{token}");
        let (tx, rx) = mpsc::unbounded_channel();
        let server = Server {
            owner: client.owner().clone(),
            window_id: window_id.clone(),
            requests: tx,
            ours: Mutex::default(),
        };
        let conn = builder.serve_at(PROMPTER_PATH, server)?.build().await?;

        if real_prompter_available(&conn).await? {
            return Ok(None);
        }
        let flags = RequestNameFlags::DoNotQueue.into();
        match conn.request_name_with_flags(PROMPTER_NAME, flags).await {
            Ok(RequestNameReply::PrimaryOwner) => {}
            _ => return Ok(None),
        }
        tracing::debug!("built-in prompter active");
        Ok(Some((Self { conn, window_id }, Requests(rx))))
    }

    /// The window identifier to pass to the provider's `Prompt` call.
    #[must_use]
    pub fn window_id(&self) -> &str {
        &self.window_id
    }
}

impl Drop for Prompter {
    fn drop(&mut self) {
        let conn = self.conn.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = conn.release_name(PROMPTER_NAME).await;
            });
        }
    }
}

/// Whether someone else owns the prompter name or can be started for it.
async fn real_prompter_available(conn: &zbus::Connection) -> Result<bool, Error> {
    let dbus = DBusProxy::new(conn).await?;
    let name = BusName::try_from(PROMPTER_NAME).map_err(|_| Error::Protocol)?;
    if dbus.name_has_owner(name).await? {
        return Ok(true);
    }
    let activatable = dbus.list_activatable_names().await?;
    Ok(activatable.iter().any(|n| n.as_str() == PROMPTER_NAME))
}

fn random_token() -> Result<String, Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| super::CryptoError::Random)?;
    Ok(bytes.iter().fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    }))
}

/// `org.gnome.keyring.internal.Prompter`, as implemented by `gcr-prompter`.
struct Server {
    owner: OwnedUniqueName,
    window_id: String,
    requests: mpsc::UnboundedSender<PasswordRequest>,
    /// Callbacks of prompts this prompter answered.
    ours: Mutex<HashSet<OwnedObjectPath>>,
}

impl Server {
    /// Only the provider Keybox is connected to may use this prompter.
    fn check_sender(&self, header: &Header<'_>) -> zbus::fdo::Result<()> {
        match header.sender() {
            Some(sender) if *sender == *self.owner => Ok(()),
            _ => Err(zbus::fdo::Error::AccessDenied(
                "not the keyring service".into(),
            )),
        }
    }
}

#[zbus::interface(name = "org.gnome.keyring.internal.Prompter")]
impl Server {
    async fn begin_prompting(
        &self,
        callback: ObjectPath<'_>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        self.check_sender(&header)?;
        // Ready for the prompt's properties. Sent after this call returns,
        // as the provider waits for the reply first.
        let proxy = callback_proxy(conn, &self.owner, callback.into()).await?;
        tokio::spawn(async move {
            let _ = proxy.prompt_ready("", HashMap::new(), "").await;
        });
        Ok(())
    }

    async fn perform_prompt(
        &self,
        callback: ObjectPath<'_>,
        prompt_type: &str,
        properties: HashMap<String, OwnedValue>,
        exchange: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        self.check_sender(&header)?;
        if prop_str(&properties, "caller-window").as_deref() != Some(self.window_id.as_str()) {
            // Another application's prompt: not Keybox's to answer.
            return Ok(());
        }
        let callback = OwnedObjectPath::from(callback);
        self.ours
            .lock()
            .expect("lock poisoned")
            .insert(callback.clone());
        let proxy = callback_proxy(conn, &self.owner, callback).await?;
        let request = (prompt_type == "password").then(|| {
            let kind = if prop_bool(&properties, "password-new") == Some(true) {
                PromptKind::NewPassword
            } else {
                PromptKind::Unlock
            };
            let retry = prop_str(&properties, "warning").is_some_and(|w| !w.is_empty());
            (kind, retry)
        });
        let requests = self.requests.clone();
        let exchange = exchange.to_owned();
        tokio::spawn(async move {
            answer(&proxy, request, &exchange, &requests).await;
        });
        Ok(())
    }

    async fn stop_prompting(
        &self,
        callback: ObjectPath<'_>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        self.check_sender(&header)?;
        let callback = OwnedObjectPath::from(callback);
        if self.ours.lock().expect("lock poisoned").remove(&callback) {
            let proxy = callback_proxy(conn, &self.owner, callback).await?;
            tokio::spawn(async move {
                let _ = proxy.prompt_done().await;
            });
        }
        Ok(())
    }
}

#[zbus::proxy(
    interface = "org.gnome.keyring.internal.Prompter.Callback",
    gen_blocking = false
)]
trait Callback {
    fn prompt_ready(
        &self,
        reply: &str,
        properties: HashMap<&str, Value<'_>>,
        exchange: &str,
    ) -> zbus::Result<()>;

    fn prompt_done(&self) -> zbus::Result<()>;
}

async fn callback_proxy(
    conn: &zbus::Connection,
    owner: &OwnedUniqueName,
    path: OwnedObjectPath,
) -> zbus::Result<CallbackProxy<'static>> {
    CallbackProxy::builder(conn)
        .destination(owner.clone())?
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// Asks Keybox's UI for the password and replies to the provider. Anything
/// that cannot be answered is cancelled, so Keybox's request never hangs.
async fn answer(
    proxy: &CallbackProxy<'static>,
    request: Option<(PromptKind, bool)>,
    exchange: &str,
    requests: &mpsc::UnboundedSender<PasswordRequest>,
) {
    let Ok(keys) = KeyPair::generate_in(Group::Ietf1536) else {
        let _ = proxy.prompt_ready("no", HashMap::new(), "").await;
        return;
    };
    let declined = exchange_text(keys.public_key(), None);
    let ready = async {
        let (kind, retry) = request?;
        let key = keys.derive_key(&parse_public(exchange)?).ok()?;
        let (reply, password) = oneshot::channel();
        requests.send(PasswordRequest { kind, retry, reply }).ok()?;
        let password = password.await.ok()??;
        encrypted_reply(&keys, &key, &password)
    };
    let result = match ready.await {
        Some(text) => proxy.prompt_ready("yes", HashMap::new(), &text).await,
        None => proxy.prompt_ready("no", HashMap::new(), &declined).await,
    };
    if result.is_err() {
        tracing::debug!("provider did not accept the prompt reply");
    }
}

fn encrypted_reply(keys: &KeyPair, key: &SessionKey, password: &str) -> Option<String> {
    let (iv, secret) = key.encrypt(password.as_bytes()).ok()?;
    Some(exchange_text(keys.public_key(), Some((&secret, &iv))))
}

/// A secret exchange message: our public key and, when answering, the
/// encrypted password and its IV.
fn exchange_text(public: &[u8], secret: Option<(&[u8], &[u8])>) -> String {
    let mut text = format!(
        "{EXCHANGE_HEADER}\npublic={}\n",
        BASE64_STANDARD.encode(public)
    );
    if let Some((secret, iv)) = secret {
        let _ = write!(
            text,
            "secret={}\niv={}\n",
            BASE64_STANDARD.encode(secret),
            BASE64_STANDARD.encode(iv)
        );
    }
    text
}

/// The provider's public key from a secret exchange message.
fn parse_public(exchange: &str) -> Option<Vec<u8>> {
    let mut lines = exchange.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next()? != EXCHANGE_HEADER {
        return None;
    }
    let value = lines.find_map(|l| l.strip_prefix("public="))?;
    BASE64_STANDARD.decode(value).ok().filter(|k| !k.is_empty())
}

/// GCR sometimes wraps property values twice; accept both forms.
fn prop<'a>(properties: &'a HashMap<String, OwnedValue>, key: &str) -> Option<&'a Value<'a>> {
    let value: &Value<'_> = properties.get(key)?;
    Some(match value {
        Value::Value(inner) => inner,
        other => other,
    })
}

fn prop_str(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    match prop(properties, key)? {
        Value::Str(s) => Some(s.as_str().to_owned()),
        _ => None,
    }
}

fn prop_bool(properties: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    match prop(properties, key)? {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_round_trip() {
        let provider = KeyPair::generate_in(Group::Ietf1536).unwrap();
        let ours = KeyPair::generate_in(Group::Ietf1536).unwrap();
        let offer = exchange_text(provider.public_key(), None);
        assert_eq!(parse_public(&offer).unwrap(), provider.public_key());

        let key = ours.derive_key(&parse_public(&offer).unwrap()).unwrap();
        let reply = encrypted_reply(&ours, &key, "KBX-UNIT-PROMPT").unwrap();
        assert!(!reply.contains("KBX-UNIT-PROMPT"));

        let field = |name: &str| {
            let line = reply
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{name}=")))
                .unwrap();
            BASE64_STANDARD.decode(line).unwrap()
        };
        let theirs = provider.derive_key(&field("public")).unwrap();
        let plain = theirs.decrypt(&field("iv"), &field("secret")).unwrap();
        assert_eq!(plain.as_slice(), b"KBX-UNIT-PROMPT");
    }

    #[test]
    fn rejects_malformed_exchanges() {
        assert_eq!(parse_public(""), None);
        assert_eq!(parse_public("public=AAAA"), None);
        assert_eq!(parse_public("[sx-aes-1]\n"), None);
        assert_eq!(parse_public("[sx-aes-1]\npublic=!!!\n"), None);
        assert_eq!(parse_public("[sx-aes-1]\npublic=AQI=\n"), Some(vec![1, 2]));
    }

    #[test]
    fn reads_single_and_double_wrapped_properties() {
        let mut props = HashMap::new();
        props.insert(
            "caller-window".to_owned(),
            OwnedValue::try_from(Value::new("wayland:keybox-1")).unwrap(),
        );
        props.insert(
            "warning".to_owned(),
            OwnedValue::try_from(Value::new(Value::new("wrong"))).unwrap(),
        );
        props.insert(
            "password-new".to_owned(),
            OwnedValue::try_from(Value::new(true)).unwrap(),
        );
        assert_eq!(
            prop_str(&props, "caller-window").as_deref(),
            Some("wayland:keybox-1")
        );
        assert_eq!(prop_str(&props, "warning").as_deref(), Some("wrong"));
        assert_eq!(prop_bool(&props, "password-new"), Some(true));
        assert_eq!(prop_str(&props, "missing"), None);
    }

    #[test]
    fn debug_never_shows_the_answer_channel() {
        let (reply, _rx) = oneshot::channel();
        let request = PasswordRequest {
            kind: PromptKind::Unlock,
            retry: false,
            reply,
        };
        assert_eq!(
            format!("{request:?}"),
            "PasswordRequest { kind: Unlock, retry: false, .. }"
        );
    }
}
