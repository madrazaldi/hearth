//! The command API the desktop app drives: one method per command.
//!
//! The app's executor is not tokio, so nothing here needs a tokio context.
//! Methods that wait (the network, the disk, the keychain) return a
//! [`Pending`], already running on the engine's own runtime, that any
//! executor can await. Dropping it does not cancel the work. The rest are
//! plain calls that only touch memory or queue a command for a connection.

use crate::certs::{self, CertInfo};
use crate::config::{Config, SaslMechanism, ServerConfig, Store};
use crate::irc::client::HistoryKind;
use crate::irc::events::{Emitter, Event};
use crate::irc::manager::Manager;
use crate::irc::message::Message;
use crate::paths;
use crate::preview::{Preview, Previewer};
use crate::runtime::{self, blocking};
use crate::secrets::{self, Keychain};
use crate::store::{Db, SavedBuffer, SearchHit, SearchQuery};
use crate::upload;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Errors reach the UI as the text it shows.
pub type CmdResult<T> = Result<T, String>;

/// Work that has already started. Awaiting it from any executor gives the
/// result; dropping it leaves the work running.
pub type Pending<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Every cause on one line, as the UI shows it.
fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn pending<T: Send + 'static>(task: JoinHandle<CmdResult<T>>) -> Pending<CmdResult<T>> {
    Box::pin(async move { task.await.unwrap_or_else(|e| Err(format!("command failed: {e}"))) })
}

fn run<T, F>(fut: F) -> Pending<CmdResult<T>>
where
    T: Send + 'static,
    F: Future<Output = CmdResult<T>> + Send + 'static,
{
    pending(runtime::spawn(fut))
}

fn failed<T: Send + 'static>(e: impl Into<String>) -> Pending<CmdResult<T>> {
    let e = e.into();
    Box::pin(async move { Err(e) })
}

/// A cheap handle: clones share one engine.
#[derive(Clone)]
pub struct Core {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    config: Mutex<Config>,
    /// Held while the config is snapshotted and written, so a slow save
    /// cannot land after a newer one.
    persist: Mutex<()>,
    irc: Arc<Manager>,
    previews: Previewer,
    config_dir: PathBuf,
    keychain: Arc<dyn Keychain>,
    close_to_tray: AtomicBool,
    load_avatars: AtomicBool,
    /// The newest queued operation per key, for [`Inner::serially`].
    ops: Mutex<HashMap<String, (u64, oneshot::Receiver<()>)>>,
    next_op: AtomicU64,
}

impl Core {
    /// Load the config from `config_dir` and open the log in `data_dir`.
    /// `sink` receives each coalesced batch of connection events, on an
    /// engine thread.
    pub fn open(config_dir: &Path, data_dir: &Path, sink: impl Fn(&[Event]) + Send + 'static) -> anyhow::Result<Core> {
        Self::open_with_keychain(config_dir, data_dir, secrets::system(), sink)
    }

    /// [`Core::open`] in the directories earlier builds of Hearth used, so an
    /// existing config and log carry over.
    pub fn open_default(sink: impl Fn(&[Event]) + Send + 'static) -> anyhow::Result<Core> {
        let dirs = paths::app_dirs();
        Self::open(&dirs.config, &dirs.data, sink)
    }

    /// [`Core::open`] with the secrets kept somewhere other than the system
    /// keychain; tests pass a [`secrets::MemoryKeychain`].
    pub fn open_with_keychain(
        config_dir: &Path,
        data_dir: &Path,
        keychain: Arc<dyn Keychain>,
        sink: impl Fn(&[Event]) + Send + 'static,
    ) -> anyhow::Result<Core> {
        let store = Store::new(config_dir, keychain.clone());
        let config = store.load();
        let db = Arc::new(Db::open(&data_dir.join("hearth.db"))?);
        let inner = Inner {
            store,
            config: Mutex::new(config.clone()),
            persist: Mutex::new(()),
            irc: Manager::new(Emitter::spawn(sink), db),
            previews: Previewer::new(),
            config_dir: config_dir.to_path_buf(),
            keychain,
            close_to_tray: AtomicBool::new(false),
            load_avatars: AtomicBool::new(false),
            ops: Mutex::new(HashMap::new()),
            next_op: AtomicU64::new(1),
        };
        inner.apply_settings(&config.settings);
        Ok(Core { inner: Arc::new(inner) })
    }

    pub fn close_to_tray(&self) -> bool {
        self.inner.close_to_tray.load(Ordering::Relaxed)
    }

    /// Quit every connection and flush the log; call before the process
    /// exits. Blocks until the log is written.
    pub fn shutdown(&self) {
        self.inner.irc.disconnect_all();
        self.inner.irc.db().flush();
    }

    /// Like [`Core::shutdown`], then closes the log (Windows cannot delete a
    /// database that is still open). Later reads fail.
    pub fn close(&self) {
        self.shutdown();
        self.inner.irc.db().close();
    }

    /// Like [`Core::shutdown`], but waits up to `max` for the QUIT lines to
    /// reach the servers. The QUITs are queued before this returns.
    pub fn shutdown_gracefully(&self, max: Duration) -> Pending<()> {
        let inner = self.inner.clone();
        inner.irc.disconnect_all();
        let task = runtime::spawn(async move {
            inner.irc.drain(max).await;
            let _ = blocking(move || inner.irc.db().flush()).await;
        });
        Box::pin(async move {
            let _ = task.await;
        })
    }

    // ---- Commands ----

    pub fn get_config(&self) -> Config {
        self.inner.config.lock().unwrap().clone()
    }

    /// Validates, stores the secrets in the keychain and writes the config.
    /// Resolves to the server as saved: id and name filled in.
    ///
    /// Saves and deletes of one server run in the order they were called, so
    /// a delete cannot be undone by a save that was slower on the keychain.
    pub fn save_server(&self, mut server: ServerConfig) -> Pending<CmdResult<ServerConfig>> {
        if server.host.trim().is_empty() {
            return failed("host is required");
        }
        if server.id.is_empty() {
            server.id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        }
        if server.name.trim().is_empty() {
            server.name = server.host.clone();
        }
        if server.sasl != SaslMechanism::Plain {
            server.sasl_password.clear();
        }
        self.inner.serially(server.id.clone(), move |inner| {
            let mut server = server;
            secrets::store(inner.keychain.as_ref(), &mut server);
            {
                let mut cfg = inner.config.lock().unwrap();
                match cfg.servers.iter_mut().find(|s| s.id == server.id) {
                    Some(existing) => *existing = server.clone(),
                    None => cfg.servers.push(server.clone()),
                }
            }
            inner.persist()?;
            Ok(server)
        })
    }

    /// Disconnects the server and removes its entry, secrets and logs.
    pub fn delete_server(&self, id: &str) -> Pending<CmdResult<()>> {
        let id = id.to_string();
        self.inner.serially(id.clone(), move |inner| {
            inner.irc.disconnect(&id, None);
            inner.config.lock().unwrap().servers.retain(|s| s.id != id);
            secrets::forget(inner.keychain.as_ref(), &id);
            if let Err(e) = inner.irc.db().forget_server(&id) {
                log::warn!("could not delete logs for {id}: {e:#}");
            }
            inner.persist()
        })
    }

    /// `settings` is the app's own JSON object, stored as it is; the
    /// engine reads only the few keys it acts on.
    pub fn save_settings(&self, settings: Value) -> CmdResult<()> {
        if !settings.is_object() {
            return Err("invalid arguments: settings must be an object".into());
        }
        self.inner.apply_settings(&settings);
        self.inner.config.lock().unwrap().settings = settings;
        self.inner.persist()
    }

    pub fn connect(&self, server: &str) -> CmdResult<()> {
        let cfg = self.inner.server(server)?;
        self.inner.irc.connect(&cfg);
        Ok(())
    }

    pub fn disconnect(&self, server: &str, reason: Option<String>) {
        self.inner.irc.disconnect(server, reason);
    }

    pub fn reconnect(&self, server: &str) {
        self.inner.irc.reconnect(server);
    }

    /// Called when the UI (re)loads: drop every live connection so the
    /// app rebuilds its state from a clean registration burst.
    pub fn reset(&self) {
        self.inner.irc.disconnect_all();
    }

    /// Queues lines for a connection. Fails when it does not exist, is closed
    /// or has too many lines waiting.
    pub fn send(&self, conn: &str, lines: Vec<String>) -> CmdResult<()> {
        self.inner.irc.send(conn, lines).map_err(err)
    }

    /// A CHATHISTORY (or search, or targets) request, answered when the
    /// server's batch completes. `kind` is a [`HistoryKind`] or one of
    /// `"messages"`, `"targets"` and `"search"`.
    pub fn history(&self, conn: &str, kind: impl Into<HistoryKind>, target: String, line: String) -> Pending<CmdResult<Vec<Message>>> {
        let (inner, conn, kind) = (self.inner.clone(), conn.to_string(), kind.into());
        run(async move { inner.irc.history(&conn, kind, target, line).await.map_err(err) })
    }

    /// Messages from the local log. `target` is the case-folded buffer name.
    pub fn local_history(&self, conn: &str, target: &str, before: Option<i64>, after: Option<i64>, limit: usize) -> Pending<CmdResult<Vec<Message>>> {
        let (conn, target) = (conn.to_string(), target.to_string());
        let limit = limit.clamp(1, 1000);
        self.inner.offload(move |inner| {
            let db = inner.irc.db();
            match after {
                Some(t) => db.history_after(&conn, &target, t, limit),
                None => db.history(&conn, &target, before, limit),
            }
            .map_err(err)
        })
    }

    pub fn search(&self, mut query: SearchQuery) -> Pending<CmdResult<Vec<SearchHit>>> {
        query.limit = query.limit.clamp(1, 500);
        self.inner.offload(move |inner| inner.irc.db().search(&query).map_err(err))
    }

    pub fn load_buffers(&self, conn: &str) -> Pending<CmdResult<Vec<SavedBuffer>>> {
        let conn = conn.to_string();
        self.inner.serially(buffers_key(&conn), move |inner| inner.irc.db().buffers(&conn).map_err(err))
    }

    /// Calls for one connection take effect in the order they were made.
    pub fn save_buffers(&self, conn: &str, buffers: Vec<SavedBuffer>) -> Pending<CmdResult<()>> {
        let conn = conn.to_string();
        self.inner.serially(buffers_key(&conn), move |inner| inner.irc.db().save_buffers(&conn, &buffers, |s| s.to_lowercase()).map_err(err))
    }

    pub fn clear_logs(&self) -> Pending<CmdResult<()>> {
        self.inner.offload(|inner| inner.irc.db().clear_logs().map_err(err))
    }

    pub fn link_preview(&self, url: &str) -> Pending<CmdResult<Preview>> {
        let (inner, url) = (self.inner.clone(), url.to_string());
        run(async move { inner.previews.preview(&url).await.map_err(err) })
    }

    /// A draft/metadata-2 avatar as a data: URL. The UI never fetches remote
    /// images itself, so the engine does (public addresses only).
    pub fn avatar(&self, url: &str) -> Pending<CmdResult<String>> {
        if !self.inner.load_avatars.load(Ordering::Relaxed) {
            return failed("avatars are turned off");
        }
        let (inner, url) = (self.inner.clone(), url.to_string());
        run(async move { inner.previews.avatar(&url).await.map_err(err) })
    }

    pub fn generate_cert(&self, server: &str) -> CmdResult<CertInfo> {
        certs::generate(&self.inner.config_dir.join("certs"), server).map_err(err)
    }

    pub fn cert_info(&self, path: &str) -> CmdResult<CertInfo> {
        certs::info(path).map_err(err)
    }

    /// Uploads a file to the server's `soju.im/FILEHOST` endpoint; resolves
    /// to the URL of the stored file.
    pub fn upload_file(&self, server: &str, url: &str, path: &str) -> Pending<CmdResult<String>> {
        let cfg = match self.inner.server(server) {
            Ok(cfg) => cfg,
            Err(e) => return failed(e),
        };
        let (url, path) = (url.to_string(), path.to_string());
        run(async move {
            let body = tokio::fs::read(&path).await.map_err(err)?;
            let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "file".into());
            upload::upload(&cfg, &url, &name, upload::guess_type(&name), body).await.map_err(err)
        })
    }

    /// Like [`Core::upload_file`], for bytes already in memory. An empty
    /// `content_type` is guessed from the name.
    pub fn upload_bytes(&self, server: &str, url: &str, name: &str, content_type: &str, data: Vec<u8>) -> Pending<CmdResult<String>> {
        let cfg = match self.inner.server(server) {
            Ok(cfg) => cfg,
            Err(e) => return failed(e),
        };
        let (url, name) = (url.to_string(), name.to_string());
        let content_type = if content_type.is_empty() { upload::guess_type(&name).to_string() } else { content_type.to_string() };
        run(async move { upload::upload(&cfg, &url, &name, &content_type, data).await.map_err(err) })
    }
}

fn buffers_key(conn: &str) -> String {
    // Cannot collide with a server id.
    format!("\0buffers:{conn}")
}

impl Inner {
    fn server(&self, id: &str) -> CmdResult<ServerConfig> {
        let cfg = self.config.lock().unwrap();
        cfg.servers.iter().find(|s| s.id == id).cloned().ok_or_else(|| "unknown server".into())
    }

    fn persist(&self) -> CmdResult<()> {
        let _writing = self.persist.lock().unwrap();
        let cfg = self.config.lock().unwrap().clone();
        self.store.save(&cfg).map_err(err)
    }

    /// Settings the engine acts on itself.
    fn apply_settings(&self, settings: &Value) {
        let flag = |k: &str, d: bool| settings.get(k).and_then(Value::as_bool).unwrap_or(d);
        self.irc.db().set_logging(flag("storeLogs", true));
        self.close_to_tray.store(flag("closeToTray", false), Ordering::Relaxed);
        self.load_avatars.store(flag("loadAvatars", false), Ordering::Relaxed);
    }

    /// Runs blocking work (disk, SQLite) off the async workers.
    fn offload<T: Send + 'static>(self: &Arc<Self>, op: impl FnOnce(&Inner) -> CmdResult<T> + Send + 'static) -> Pending<CmdResult<T>> {
        let inner = self.clone();
        run(async move { blocking(move || op(&inner)).await.and_then(|r| r) })
    }

    /// Like `offload`, but operations with the same `key` run one after the
    /// other in the order they were called, whatever order the runtime
    /// happens to start them in. The place in line is taken now, not when
    /// the future is first polled.
    fn serially<T: Send + 'static>(self: &Arc<Self>, key: String, op: impl FnOnce(&Inner) -> CmdResult<T> + Send + 'static) -> Pending<CmdResult<T>> {
        let (done, done_rx) = oneshot::channel::<()>();
        let ticket = self.next_op.fetch_add(1, Ordering::Relaxed);
        let before = self.ops.lock().unwrap().insert(key.clone(), (ticket, done_rx)).map(|(_, rx)| rx);
        let inner = self.clone();
        run(async move {
            if let Some(before) = before {
                // Resolves with an error when the earlier one is dropped; either way it is over.
                let _ = before.await;
            }
            let worker = inner.clone();
            let result = blocking(move || op(&worker)).await.and_then(|r| r);
            {
                let mut ops = inner.ops.lock().unwrap();
                if ops.get(&key).is_some_and(|(t, _)| *t == ticket) {
                    ops.remove(&key);
                }
            }
            let _ = done.send(());
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryKeychain;
    use crate::test_util::TempDir;
    use serde_json::json;

    struct Fixture {
        dir: TempDir,
        vault: Arc<MemoryKeychain>,
        cores: Vec<Core>,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture { dir: TempDir::new(), vault: Arc::new(MemoryKeychain::default()), cores: Vec::new() }
        }

        fn open(&mut self) -> Core {
            let core = Core::open_with_keychain(self.dir.path(), self.dir.path(), self.vault.clone(), |_| {}).unwrap();
            self.cores.push(core.clone());
            core
        }

        fn secret(&self, name: &str) -> Option<String> {
            self.vault.get(secrets::SERVICE, name).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for core in &self.cores {
                core.close();
            }
        }
    }

    fn server(host: &str, nick: &str, password: &str) -> ServerConfig {
        ServerConfig { host: host.into(), nick: nick.into(), password: password.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn settings_and_config_round_trip() {
        let mut fx = Fixture::new();
        let core = fx.open();
        core.save_settings(json!({"storeLogs": false, "theme": "dark"})).unwrap();
        let cfg = core.get_config();
        assert_eq!(cfg.settings["theme"], "dark");
        assert!(cfg.servers.is_empty());
        assert_eq!(core.connect("nope").unwrap_err(), "unknown server");
        assert!(core.send("nope", vec![]).is_err());
        assert!(core.search(SearchQuery { text: "x".into(), limit: 5, ..Default::default() }).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn settings_must_be_an_object() {
        let mut fx = Fixture::new();
        let core = fx.open();
        for bad in [json!(null), json!([1]), json!("dark")] {
            assert!(core.save_settings(bad).unwrap_err().contains("must be an object"));
        }
        assert!(core.get_config().settings.as_object().unwrap().is_empty());
    }

    #[tokio::test]
    async fn saved_buffers_round_trip() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let bufs: Vec<SavedBuffer> =
            serde_json::from_value(json!([{"name": "#Hearth", "kind": "channel", "key": ""}, {"name": "alice", "kind": "query", "key": ""}])).unwrap();
        core.save_buffers("c", bufs).await.unwrap();
        assert_eq!(core.load_buffers("c").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn buffer_saves_land_in_call_order() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let list =
            |n: usize| -> Vec<SavedBuffer> { (0..n).map(|i| SavedBuffer { name: format!("#c{i}"), kind: "channel".into(), key: String::new() }).collect() };
        let saves: Vec<_> = (1..=40).map(|n| core.save_buffers("c", list(n))).collect();
        let last = core.load_buffers("c");
        for s in saves {
            s.await.unwrap();
        }
        assert_eq!(last.await.unwrap().len(), 40);
    }

    #[tokio::test]
    async fn saving_a_server_fills_in_its_id_and_name_and_keeps_secrets_out_of_the_file() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let input =
            ServerConfig { sasl: SaslMechanism::Plain, sasl_username: "a".into(), sasl_password: "sp".into(), ..server("irc.example", "me", "hunter2") };
        let saved = core.save_server(input).await.unwrap();
        assert_eq!(saved.id.len(), 12);
        assert!(saved.id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{}", saved.id);
        assert_eq!(saved.name, "irc.example");
        assert!(!serde_json::to_string(&saved).unwrap().contains("inKeychain"));
        assert_eq!(fx.secret(&format!("{}:password", saved.id)).as_deref(), Some("hunter2"));
        assert_eq!(fx.secret(&format!("{}:sasl", saved.id)).as_deref(), Some("sp"));
        let on_disk = std::fs::read_to_string(fx.dir.path().join("config.json")).unwrap();
        assert!(!on_disk.contains("hunter2") && !on_disk.contains("\"sp\""), "{on_disk}");
        assert_eq!(core.get_config().servers[0].password, "hunter2");

        // A SASL password only counts for PLAIN.
        let other = core.save_server(ServerConfig { sasl: SaslMechanism::None, sasl_password: "ignored".into(), ..saved.clone() }).await.unwrap();
        assert_eq!(other.sasl_password, "");

        // Saving again updates in place.
        core.save_server(ServerConfig { nick: "renamed".into(), ..saved }).await.unwrap();
        let servers = core.get_config().servers;
        assert_eq!((servers.len(), servers[0].nick.as_str()), (1, "renamed"));
    }

    #[tokio::test]
    async fn a_server_needs_a_host() {
        let mut fx = Fixture::new();
        let core = fx.open();
        assert_eq!(core.save_server(server("  ", "n", "")).await.unwrap_err(), "host is required");
    }

    #[tokio::test]
    async fn deleting_a_server_removes_its_secrets_logs_and_entry() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let saved = core.save_server(server("h", "n", "pw")).await.unwrap();
        core.save_buffers(&saved.id, vec![SavedBuffer { name: "#a".into(), kind: "channel".into(), key: String::new() }]).await.unwrap();
        core.delete_server(&saved.id).await.unwrap();
        assert!(core.get_config().servers.is_empty());
        assert!(fx.vault.is_empty());
        assert!(core.load_buffers(&saved.id).await.unwrap().is_empty());
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(fx.dir.path().join("config.json")).unwrap()).unwrap();
        assert_eq!(on_disk["servers"], json!([]));
    }

    #[tokio::test]
    async fn a_delete_during_a_save_is_not_undone_by_it() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let saved = core.save_server(ServerConfig { id: "x".into(), ..server("h", "n", "pw") }).await.unwrap();
        // Both are called before either finishes its keychain work; the
        // delete is called second, so it must also finish second.
        for _ in 0..50 {
            let save = core.save_server(ServerConfig { password: "pw2".into(), ..saved.clone() });
            let delete = core.delete_server("x");
            let (saved_again, deleted) = (save.await, delete.await);
            assert!(saved_again.is_ok() && deleted.is_ok());
            assert!(core.get_config().servers.is_empty());
            assert!(fx.vault.is_empty());
            core.save_server(saved.clone()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn the_engine_applies_the_settings_it_acts_on() {
        let mut fx = Fixture::new();
        let core = fx.open();
        assert!(!core.close_to_tray());
        core.save_settings(json!({"closeToTray": true})).unwrap();
        assert!(core.close_to_tray());
        // Avatars stay off unless the user opted in.
        assert_eq!(core.avatar("https://example.com/a.png").await.unwrap_err(), "avatars are turned off");
    }

    #[tokio::test]
    async fn the_config_survives_a_restart() {
        let mut fx = Fixture::new();
        let a = fx.open();
        a.save_server(ServerConfig { id: "x".into(), ..server("h", "n", "pw") }).await.unwrap();
        a.save_settings(json!({"theme": "light"})).unwrap();
        a.close();
        let b = fx.open();
        assert_eq!(b.get_config().servers[0].password, "pw");
        assert_eq!(b.get_config().settings, json!({"theme": "light"}));
    }

    #[tokio::test]
    async fn secrets_the_earlier_engine_stored_are_found_and_moved_over() {
        let mut fx = Fixture::new();
        let a = fx.open();
        a.save_server(ServerConfig { id: "x".into(), ..server("h", "n", "pw") }).await.unwrap();
        a.close();
        fx.vault.demote_all();
        let b = fx.open();
        assert_eq!(b.get_config().servers[0].password, "pw");
        assert_eq!(fx.secret("x:password").as_deref(), Some("pw"));
        // The old entry is gone, so clearing the password sticks.
        assert_eq!(fx.vault.legacy_len(), 0);
        let cleared = ServerConfig { password: String::new(), ..b.get_config().servers[0].clone() };
        b.save_server(cleared).await.unwrap();
        b.close();
        let c = fx.open();
        assert_eq!(c.get_config().servers[0].password, "");
    }

    #[tokio::test]
    async fn local_history_limits_are_clamped_and_results_come_back() {
        let mut fx = Fixture::new();
        let core = fx.open();
        assert!(core.local_history("c", "#a", None, None, 0).await.unwrap().is_empty());
        assert!(core.local_history("c", "#a", None, Some(5), 100_000).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn certificates_are_generated_under_the_config_directory() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let info = core.generate_cert("my-server").unwrap();
        assert_eq!(PathBuf::from(&info.path), fx.dir.path().join("certs").join("my-server.pem"));
        assert!(Path::new(&info.path).exists());
        assert_eq!(core.cert_info(&info.path).unwrap().sha256, info.sha256);
        assert!(core.generate_cert("///").is_err());
    }

    #[tokio::test]
    async fn uploads_need_a_known_server() {
        let mut fx = Fixture::new();
        let core = fx.open();
        assert_eq!(core.upload_bytes("nope", "https://x", "a.txt", "", b"hi".to_vec()).await.unwrap_err(), "unknown server");
        assert_eq!(core.upload_file("nope", "https://x", "/a.txt").await.unwrap_err(), "unknown server");
    }

    #[tokio::test]
    async fn history_needs_a_live_connection() {
        let mut fx = Fixture::new();
        let core = fx.open();
        let err = core.history("nope", "messages", "#a".into(), "CHATHISTORY LATEST #a * 10".into()).await.unwrap_err();
        assert_eq!(err, "no such connection");
    }

    #[tokio::test]
    async fn clearing_logs_leaves_saved_buffers_alone() {
        let mut fx = Fixture::new();
        let core = fx.open();
        core.save_buffers("c", vec![SavedBuffer { name: "#a".into(), kind: "channel".into(), key: String::new() }]).await.unwrap();
        core.clear_logs().await.unwrap();
        assert_eq!(core.load_buffers("c").await.unwrap().len(), 1);
    }
}
