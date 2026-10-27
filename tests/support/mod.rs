// SPDX-License-Identifier: GPL-3.0-only

//! Isolated Secret Service test environment.
//!
//! Every test gets its own private `dbus-daemon` (with no service activation,
//! so it can never start a real provider) and its own `oo7-daemon` whose data,
//! config, cache and runtime directories live in a temporary directory. The
//! user's session bus and keyrings are never touched: nothing here reads
//! `DBUS_SESSION_BUS_ADDRESS`, and child processes get a cleared environment.

#![allow(dead_code, clippy::pedantic)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use keybox::backend::Bus;
use oo7::Secret;
use oo7::file::UnlockedKeyring;
use tempfile::TempDir;

pub const LOGIN_PASSWORD: &str = "kbx-test-login-password";
pub const WORK_PASSWORD: &str = "kbx-test-work-password";

/// Distinctive synthetic values. Tests assert none of these leak.
pub const SECRET_ALPHA: &str = "KBX-FIXTURE-SECRET-ALPHA";
pub const SECRET_CONTROL: &str = "KBX-FIXTURE\nline2\0end";
pub const SECRET_BLOB_TEXT: &[u8] = b"KBX-FIXTURE-BLOB-TEXT";
pub const SECRET_BINARY: &[u8] = &[0xff, 0xfe, 0x00, 0x4b];
pub const LARGE_LABEL_LEN: usize = 10_000;
pub const LARGE_ATTRIBUTE_COUNT: usize = 200;

pub struct Fixture {
    pub label: String,
    pub attributes: HashMap<String, String>,
    pub secret: Secret,
    /// Content type to declare instead of the one implied by `secret`
    /// (D-Bus fixtures only).
    pub content_type: Option<String>,
}

fn fixture(label: &str, attrs: &[(&str, &str)], secret: Secret) -> Fixture {
    Fixture {
        label: label.to_owned(),
        attributes: attrs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .chain([("kbx-test".to_owned(), "1".to_owned())])
            .collect(),
        secret,
        content_type: None,
    }
}

/// The standard login keyring contents.
#[must_use]
pub fn login_fixtures() -> Vec<Fixture> {
    let large_attrs: Vec<(String, String)> = (0..LARGE_ATTRIBUTE_COUNT)
        .map(|i| (format!("kbx-attr-{i:03}"), "v".repeat(100)))
        .collect();
    let large_refs: Vec<(&str, &str)> = large_attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    vec![
        fixture(
            "KBX Alpha",
            &[
                ("kind", "alpha"),
                ("user", "alice"),
                ("url", "https://example.invalid"),
            ],
            Secret::text(SECRET_ALPHA),
        ),
        fixture(
            "KBX Duplicate",
            &[("kind", "dup"), ("n", "1")],
            Secret::text("KBX-FIXTURE-DUP-1"),
        ),
        fixture(
            "KBX Duplicate",
            &[("kind", "dup"), ("n", "2")],
            Secret::text("KBX-FIXTURE-DUP-2"),
        ),
        fixture(
            "",
            &[("kind", "nolabel")],
            Secret::text("KBX-FIXTURE-NOLABEL"),
        ),
        fixture("KBX Empty", &[("kind", "empty")], Secret::text("")),
        fixture(
            "KBX Control",
            &[("kind", "control")],
            Secret::text(SECRET_CONTROL),
        ),
        fixture(
            "KBX Binary",
            &[("kind", "binary")],
            Secret::blob(SECRET_BINARY),
        ),
        fixture(
            "KBX Blob text",
            &[("kind", "blobtext")],
            Secret::blob(SECRET_BLOB_TEXT),
        ),
        fixture(
            &"L".repeat(LARGE_LABEL_LEN),
            &[&[("kind", "large")][..], &large_refs].concat(),
            Secret::text("KBX-FIXTURE-LARGE"),
        ),
        fixture(
            "KBX <b>Markup</b> & <a href='https://example.invalid'>link</a>",
            &[("kind", "markup")],
            Secret::text("KBX-FIXTURE-MARKUP"),
        ),
    ]
}

/// Contents of a second keyring with a different password. oo7-daemon
/// discovers it locked.
#[must_use]
pub fn work_fixtures() -> Vec<Fixture> {
    vec![fixture(
        "KBX Work",
        &[("kind", "work")],
        Secret::text("KBX-FIXTURE-WORK"),
    )]
}

pub struct TestEnv {
    dir: TempDir,
    bus: Child,
    daemon: Option<Child>,
    address: String,
    prompter: Option<(zbus::Connection, FakePrompter)>,
}

/// Returns `None` (and the caller skips the test) when the required system
/// binaries are missing.
pub fn tools_available() -> bool {
    let ok = which("dbus-daemon").is_some() && oo7_daemon().is_some();
    if !ok {
        eprintln!("SKIPPED: dbus-daemon or oo7-daemon not installed");
    }
    ok
}

impl TestEnv {
    /// Starts a private bus without any provider.
    pub fn bus_only() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("keybox-test-")
            .tempdir()
            .unwrap();
        for sub in [
            "data/keyrings/v1",
            "config",
            "cache",
            "runtime",
            "home",
            "services",
        ] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        set_mode(&dir.path().join("runtime"), 0o700);

        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}/bus</listen>
  <auth>EXTERNAL</auth>
  <servicedir>{}/services</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                dir.path().display(),
                dir.path().display()
            ),
        )
        .unwrap();

        let mut bus = Command::new(which("dbus-daemon").unwrap())
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let address = line.trim().to_owned();
        assert!(address.starts_with("unix:"), "bad bus address {address:?}");

        Self {
            dir,
            bus,
            daemon: None,
            address,
            prompter: None,
        }
    }

    /// Starts a private bus plus oo7-daemon with the standard fixtures. The
    /// login keyring is unlocked with its password on stdin; the work keyring
    /// stays locked.
    pub async fn oo7() -> Self {
        let mut env = Self::bus_only();
        env.write_keyring("login", LOGIN_PASSWORD, login_fixtures())
            .await;
        env.write_keyring("work", WORK_PASSWORD, work_fixtures())
            .await;
        env.start_oo7().await;
        env
    }

    pub fn bus(&self) -> Bus {
        Bus::Address(self.address.clone())
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// Writes a keyring file for oo7-daemon to discover at startup.
    pub async fn write_keyring(&self, name: &str, password: &str, items: Vec<Fixture>) {
        let path = self
            .dir
            .path()
            .join(format!("data/keyrings/v1/{name}.keyring"));
        let keyring = UnlockedKeyring::load(&path, Secret::text(password))
            .await
            .unwrap();
        for item in items {
            keyring
                .create_item(&item.label, &item.attributes, item.secret, false)
                .await
                .unwrap();
        }
    }

    /// Like [`TestEnv::oo7`], plus a fake KDE prompter that answers unlock
    /// prompts according to `behaviour`.
    pub async fn oo7_with_prompter(behaviour: PromptBehaviour) -> (Self, FakePrompter) {
        let mut env = Self::bus_only();
        env.write_keyring("login", LOGIN_PASSWORD, login_fixtures())
            .await;
        env.write_keyring("work", WORK_PASSWORD, work_fixtures())
            .await;
        // oo7-daemon only uses the KDE prompter if the name is activatable.
        std::fs::write(
            env.dir
                .path()
                .join("services/org.kde.secretprompter.service"),
            "[D-BUS Service]\nName=org.kde.secretprompter\nExec=/bin/false\n",
        )
        .unwrap();
        let prompter = FakePrompter::new(behaviour);
        let conn = zbus::connection::Builder::address(env.address.as_str())
            .unwrap()
            .name("org.kde.secretprompter")
            .unwrap()
            .serve_at("/SecretPrompter", prompter.clone())
            .unwrap()
            .build()
            .await
            .unwrap();
        env.prompter = Some((conn, prompter.clone()));
        env.start_oo7().await;
        (env, prompter)
    }

    pub async fn start_oo7(&mut self) {
        assert!(self.daemon.is_none(), "oo7-daemon already running");
        let d = self.dir.path();
        let desktop = if self.prompter.is_some() { "KDE" } else { "" };
        let mut child = Command::new(oo7_daemon().unwrap())
            .arg("--login")
            .env_clear()
            .env("XDG_CURRENT_DESKTOP", desktop)
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", d.join("home"))
            .env("XDG_DATA_HOME", d.join("data"))
            .env("XDG_CONFIG_HOME", d.join("config"))
            .env("XDG_CACHE_HOME", d.join("cache"))
            .env("XDG_RUNTIME_DIR", d.join("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                std::fs::File::create(d.join("oo7-daemon.log")).unwrap(),
            ))
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(LOGIN_PASSWORD.as_bytes())
            .unwrap();
        self.daemon = Some(child);
        self.wait_for_owner(true).await;
        self.wait_for_oo7_ready().await;
    }

    /// oo7-daemon 0.6 owns its bus name before it has loaded its keyrings:
    /// until then it lists no collections, and some calls make it panic
    /// (`Service::connection` is still unset). It registers the `session`
    /// collection last, so wait for that object to exist.
    ///
    /// Checked by introspection: reading `Collections` during start-up can
    /// deadlock the daemon (the property waits for a lock that start-up
    /// holds while it registers objects).
    async fn wait_for_oo7_ready(&self) {
        const SESSION: &str = "/org/freedesktop/secrets/collection/session";
        let conn = self.raw_connection().await;
        for _ in 0..200 {
            let reply = tokio::time::timeout(
                Duration::from_secs(2),
                conn.call_method(
                    Some("org.freedesktop.secrets"),
                    SESSION,
                    Some("org.freedesktop.DBus.Introspectable"),
                    "Introspect",
                    &(),
                ),
            )
            .await;
            if let Ok(Ok(reply)) = reply
                && let Ok(xml) = reply.body().deserialize::<String>()
                && xml.contains("org.freedesktop.Secret.Collection")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "oo7-daemon did not finish starting; daemon log:\n{}",
            self.daemon_log()
        );
    }

    /// Starts a private bus plus GNOME Keyring, with the standard fixtures
    /// created over D-Bus in its login keyring. Returns `None` (skip) when no
    /// `gnome-keyring-daemon` is available; see [`gnome_keyring_daemon`].
    pub async fn gnome_keyring() -> Option<Self> {
        let (daemon, lib_dir) = gnome_keyring_daemon()?;
        let mut env = Self::bus_only();
        let d = env.dir.path().to_owned();
        let mut command = Command::new(daemon);
        command
            .args(["--foreground", "--components=secrets", "--unlock"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", d.join("home"))
            .env("XDG_DATA_HOME", d.join("data"))
            .env("XDG_CONFIG_HOME", d.join("config"))
            .env("XDG_CACHE_HOME", d.join("cache"))
            .env("XDG_RUNTIME_DIR", d.join("runtime"))
            .env("DBUS_SESSION_BUS_ADDRESS", &env.address)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                std::fs::File::create(d.join("oo7-daemon.log")).unwrap(),
            ));
        if let Some(lib_dir) = lib_dir {
            command.env("LD_LIBRARY_PATH", lib_dir);
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(LOGIN_PASSWORD.as_bytes())
            .unwrap();
        env.daemon = Some(child);
        env.wait_for_owner(true).await;

        let mut fixtures = login_fixtures();
        fixtures.push(Fixture {
            content_type: Some("text/plain; charset=utf8".into()),
            ..fixture(
                "KBX Charset",
                &[("kind", "charset")],
                Secret::text("KBX-FIXTURE-CHARSET"),
            )
        });
        env.create_items("/org/freedesktop/secrets/aliases/default", fixtures)
            .await;
        Some(env)
    }

    /// Creates items over D-Bus with a plain (test-only) session.
    pub async fn create_items(&self, collection: &str, items: Vec<Fixture>) {
        use zbus::zvariant::{OwnedObjectPath, Value};
        let conn = self.raw_connection().await;
        let service = "org.freedesktop.secrets";
        let (_, session): (zbus::zvariant::OwnedValue, OwnedObjectPath) = conn
            .call_method(
                Some(service),
                "/org/freedesktop/secrets",
                Some("org.freedesktop.Secret.Service"),
                "OpenSession",
                &("plain", Value::from("")),
            )
            .await
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        for item in items {
            let content_type = item
                .content_type
                .clone()
                .unwrap_or_else(|| item.secret.content_type().as_str().to_owned());
            let mut props: HashMap<&str, Value<'_>> = HashMap::new();
            props.insert(
                "org.freedesktop.Secret.Item.Label",
                Value::from(item.label.as_str()),
            );
            props.insert(
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(item.attributes.clone()),
            );
            let secret = (
                session.clone(),
                Vec::<u8>::new(),
                item.secret.as_bytes().to_vec(),
                content_type,
            );
            let reply = conn
                .call_method(
                    Some(service),
                    collection,
                    Some("org.freedesktop.Secret.Collection"),
                    "CreateItem",
                    &(props, secret, false),
                )
                .await
                .unwrap();
            let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
                reply.body().deserialize().unwrap();
            assert_eq!(prompt.as_str(), "/", "creating a fixture needed a prompt");
        }
    }

    /// Stops the provider daemon (oo7 or GNOME Keyring).
    pub async fn stop_oo7(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.wait_for_owner(false).await;
    }

    /// Installs a D-Bus service file for `org.freedesktop.secrets` that
    /// fails to start, to exercise activation errors.
    pub fn install_failing_activation(&self) {
        std::fs::write(
            self.dir
                .path()
                .join("services/org.freedesktop.secrets.service"),
            "[D-BUS Service]\nName=org.freedesktop.secrets\nExec=/bin/false\n",
        )
        .unwrap();
    }

    pub fn daemon_log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("oo7-daemon.log")).unwrap_or_default()
    }

    /// A plain connection to the private bus, for test-side calls.
    pub async fn raw_connection(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }

    async fn wait_for_owner(&self, present: bool) {
        let conn = self.raw_connection().await;
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
        for _ in 0..100 {
            let has = dbus
                .name_has_owner("org.freedesktop.secrets".try_into().unwrap())
                .await
                .unwrap();
            if has == present {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "org.freedesktop.secrets owner present={present} not reached; daemon log:\n{}",
            self.daemon_log()
        );
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = self.bus.kill();
        let _ = self.bus.wait();
    }
}

fn which(name: &str) -> Option<PathBuf> {
    ["/usr/bin", "/bin", "/usr/local/bin"]
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|p| p.is_file())
}

/// A `gnome-keyring-daemon` to test against: the installed one, or one from
/// an unpacked package root in `KEYBOX_TEST_GNOME_KEYRING_ROOT` (with its
/// `usr/lib` for libraries). gnome-keyring conflicts with oo7 on Arch, so
/// the unpacked form avoids replacing the real provider.
pub fn gnome_keyring_daemon() -> Option<(PathBuf, Option<PathBuf>)> {
    if let Ok(root) = std::env::var("KEYBOX_TEST_GNOME_KEYRING_ROOT") {
        let root = PathBuf::from(root);
        let daemon = root.join("usr/bin/gnome-keyring-daemon");
        if daemon.is_file() {
            return Some((daemon, Some(root.join("usr/lib"))));
        }
    }
    if let Some(daemon) = which("gnome-keyring-daemon") {
        return Some((daemon, None));
    }
    eprintln!("SKIPPED: no gnome-keyring-daemon (set KEYBOX_TEST_GNOME_KEYRING_ROOT)");
    None
}

fn oo7_daemon() -> Option<PathBuf> {
    which("oo7-daemon").or_else(|| {
        let p = PathBuf::from("/usr/lib/oo7-daemon");
        p.is_file().then_some(p)
    })
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// What the fake prompter does when asked to unlock.
#[derive(Debug, Clone)]
pub enum PromptBehaviour {
    /// Answer with these passwords in turn (wrong ones trigger a retry).
    Accept(Vec<String>),
    /// Press "Cancel".
    Reject,
    /// Never answer, like a desktop with no prompter UI.
    Ignore,
}

/// A stand-in for KDE's `org.kde.secretprompter`, which oo7-daemon calls to
/// show unlock dialogs. Records the window identifiers it receives.
#[derive(Clone)]
pub struct FakePrompter {
    behaviour: PromptBehaviour,
    pub window_ids: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    pub calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl FakePrompter {
    fn new(behaviour: PromptBehaviour) -> Self {
        Self {
            behaviour,
            window_ids: std::sync::Arc::default(),
            calls: std::sync::Arc::default(),
        }
    }
}

#[zbus::proxy(interface = "org.kde.secretprompter.request", gen_blocking = false)]
trait PromptRequest {
    fn accepted(&self, result_fd: zbus::zvariant::Fd<'_>) -> zbus::Result<i32>;
    fn rejected(&self) -> zbus::Result<i32>;
}

#[zbus::interface(name = "org.kde.secretprompter")]
impl FakePrompter {
    async fn unlock_collection_prompt(
        &self,
        request: zbus::zvariant::OwnedObjectPath,
        window_id: String,
        _activation_token: String,
        _collection_name: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) {
        self.answer(request, window_id, &header, conn);
    }

    /// Asked when a new keyring needs a password.
    async fn create_collection_prompt(
        &self,
        request: zbus::zvariant::OwnedObjectPath,
        window_id: String,
        _activation_token: String,
        _collection_name: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) {
        self.answer(request, window_id, &header, conn);
    }
}

impl FakePrompter {
    fn answer(
        &self,
        request: zbus::zvariant::OwnedObjectPath,
        window_id: String,
        header: &zbus::message::Header<'_>,
        conn: &zbus::Connection,
    ) {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.window_ids.lock().unwrap().push(window_id);
        let sender = header.sender().unwrap().to_owned();
        let conn = conn.clone();
        let behaviour = self.behaviour.clone();
        // Answer after replying, as the real prompter does.
        tokio::spawn(async move {
            let proxy = PromptRequestProxy::builder(&conn)
                .destination(sender)
                .unwrap()
                .path(request)
                .unwrap()
                .build()
                .await
                .unwrap();
            match behaviour {
                PromptBehaviour::Ignore => {}
                PromptBehaviour::Reject => {
                    let _ = proxy.rejected().await;
                }
                PromptBehaviour::Accept(passwords) => {
                    for password in passwords {
                        let (mut ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
                        ours.write_all(password.as_bytes()).unwrap();
                        drop(ours);
                        use std::os::fd::AsFd;
                        let keep = proxy.accepted(theirs.as_fd().into()).await;
                        // 0 = dismiss (done), 1 = keep (retry with the next password).
                        if !matches!(keep, Ok(1)) {
                            break;
                        }
                    }
                }
            }
        });
    }
}

/// Counts `GetSecret`/`GetSecrets` calls on the private bus by becoming a
/// bus monitor. Used to prove that browsing never retrieves secrets.
pub async fn secret_call_counter(env: &TestEnv) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    use futures_util::StreamExt;
    let conn = env.raw_connection().await;
    let rules = vec![
        "type='method_call',interface='org.freedesktop.Secret.Item',member='GetSecret'",
        "type='method_call',interface='org.freedesktop.Secret.Service',member='GetSecrets'",
    ];
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus.Monitoring"),
        "BecomeMonitor",
        &(rules, 0u32),
    )
    .await
    .unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = count.clone();
    let mut stream = zbus::MessageStream::from(&conn);
    tokio::spawn(async move {
        let _keep = conn;
        while let Some(Ok(msg)) = stream.next().await {
            if let Some(member) = msg.header().member()
                && matches!(member.as_str(), "GetSecret" | "GetSecrets")
            {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
    // Let the monitor become active before the test proceeds.
    tokio::time::sleep(Duration::from_millis(100)).await;
    count
}

/// Records the raw bytes of `CreateItem` and `SetSecret` calls on the
/// private bus, to prove that secrets are never sent in plaintext.
pub async fn write_call_recorder(env: &TestEnv) -> std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>> {
    use futures_util::StreamExt;
    let conn = env.raw_connection().await;
    let rules = vec![
        "type='method_call',interface='org.freedesktop.Secret.Collection',member='CreateItem'",
        "type='method_call',interface='org.freedesktop.Secret.Item',member='SetSecret'",
    ];
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus.Monitoring"),
        "BecomeMonitor",
        &(rules, 0u32),
    )
    .await
    .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let mut stream = zbus::MessageStream::from(&conn);
    tokio::spawn(async move {
        let _keep = conn;
        while let Some(Ok(msg)) = stream.next().await {
            if let Some(member) = msg.header().member()
                && matches!(member.as_str(), "CreateItem" | "SetSecret")
            {
                recorder.lock().unwrap().push(msg.data().to_vec());
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    seen
}

/// Whether `haystack` contains `needle` as a byte sequence.
pub fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
