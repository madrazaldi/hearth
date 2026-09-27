//! The command API every frontend uses. Tauri wraps each method in a
//! `#[tauri::command]`; the macOS app reaches them through [`Core::call`], a
//! JSON dispatcher whose command and argument names match the Tauri ones.

use crate::certs::{self, CertInfo};
use crate::config::{Config, SaslMechanism, ServerConfig, Store};
use crate::irc::client::HistoryKind;
use crate::irc::events::{Emitter, Event};
use crate::irc::manager::Manager;
use crate::irc::message::Message;
use crate::preview::{Preview, Previewer};
use crate::runtime::blocking;
use crate::secrets;
use crate::store::{Db, SavedBuffer, SearchHit};
use crate::upload;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct Core {
    store: Store,
    config: Mutex<Config>,
    irc: Arc<Manager>,
    previews: Previewer,
    config_dir: PathBuf,
    close_to_tray: AtomicBool,
    load_avatars: AtomicBool,
}

impl Core {
    /// Load the config from `config_dir` and open the log in `data_dir`.
    /// `sink` receives each coalesced batch of connection events.
    pub fn open(config_dir: &Path, data_dir: &Path, sink: impl Fn(&[Event]) + Send + 'static) -> anyhow::Result<Core> {
        let store = Store::new(config_dir);
        let config = store.load();
        let db = Arc::new(Db::open(&data_dir.join("hearth.db"))?);
        let core = Core {
            store,
            config: Mutex::new(config.clone()),
            irc: Manager::new(Emitter::spawn(sink), db),
            previews: Previewer::new(),
            config_dir: config_dir.to_path_buf(),
            close_to_tray: AtomicBool::new(false),
            load_avatars: AtomicBool::new(false),
        };
        core.apply_settings(&config.settings);
        Ok(core)
    }

    fn server(&self, id: &str) -> CmdResult<ServerConfig> {
        let cfg = self.config.lock().unwrap();
        cfg.servers.iter().find(|s| s.id == id).cloned().ok_or_else(|| "unknown server".into())
    }

    fn persist(&self) -> CmdResult<()> {
        let cfg = self.config.lock().unwrap().clone();
        self.store.save(&cfg).map_err(err)
    }

    fn db(&self) -> &Arc<Db> {
        self.irc.db()
    }

    /// Settings the backend acts on itself.
    fn apply_settings(&self, settings: &Value) {
        let flag = |k: &str, d: bool| settings.get(k).and_then(Value::as_bool).unwrap_or(d);
        self.db().set_logging(flag("storeLogs", true));
        self.close_to_tray.store(flag("closeToTray", false), Ordering::Relaxed);
        self.load_avatars.store(flag("loadAvatars", false), Ordering::Relaxed);
    }

    pub fn close_to_tray(&self) -> bool {
        self.close_to_tray.load(Ordering::Relaxed)
    }

    /// Quit every connection and flush the log; call before the process exits.
    pub fn shutdown(&self) {
        self.irc.disconnect_all();
        self.db().flush();
    }

    pub fn get_config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn save_server(&self, mut server: ServerConfig) -> CmdResult<ServerConfig> {
        if server.host.trim().is_empty() {
            return Err("host is required".into());
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
        secrets::store(&mut server);
        {
            let mut cfg = self.config.lock().unwrap();
            match cfg.servers.iter_mut().find(|s| s.id == server.id) {
                Some(existing) => *existing = server.clone(),
                None => cfg.servers.push(server.clone()),
            }
        }
        self.persist()?;
        Ok(server)
    }

    pub fn delete_server(&self, id: &str) -> CmdResult<()> {
        self.irc.disconnect(id, None);
        self.config.lock().unwrap().servers.retain(|s| s.id != id);
        secrets::forget(id);
        if let Err(e) = self.db().forget_server(id) {
            log::warn!("could not delete logs for {id}: {e}");
        }
        self.persist()
    }

    pub fn save_settings(&self, settings: Value) -> CmdResult<()> {
        self.apply_settings(&settings);
        self.config.lock().unwrap().settings = settings;
        self.persist()
    }

    pub fn connect(&self, server: &str) -> CmdResult<()> {
        let cfg = self.server(server)?;
        self.irc.connect(&cfg);
        Ok(())
    }

    pub fn disconnect(&self, server: &str, reason: Option<String>) {
        self.irc.disconnect(server, reason);
    }

    pub fn reconnect(&self, server: &str) {
        self.irc.reconnect(server);
    }

    /// Called when the UI (re)loads: drop every live connection so the frontend
    /// rebuilds its state from a clean registration burst.
    pub fn reset(&self) {
        self.irc.disconnect_all();
    }

    pub fn send(&self, conn: &str, lines: Vec<String>) -> CmdResult<()> {
        self.irc.send(conn, lines).map_err(err)
    }

    pub async fn history(&self, conn: &str, kind: &str, target: String, line: String) -> CmdResult<Vec<Message>> {
        let kind = match kind {
            "targets" => HistoryKind::Targets,
            "search" => HistoryKind::Search,
            _ => HistoryKind::Messages,
        };
        self.irc.history(conn, kind, target, line).await.map_err(err)
    }

    /// Messages from the local log. `target` is the case-folded buffer name.
    pub async fn local_history(
        &self,
        conn: String,
        target: String,
        before: Option<i64>,
        after: Option<i64>,
        limit: usize,
    ) -> CmdResult<Vec<Message>> {
        let db = self.db().clone();
        let limit = limit.clamp(1, 1000);
        blocking(move || match after {
            Some(t) => db.history_after(&conn, &target, t, limit),
            None => db.history(&conn, &target, before, limit),
        })
        .await?
        .map_err(err)
    }

    pub async fn search(
        &self,
        query: String,
        network: Option<String>,
        target: Option<String>,
        limit: usize,
    ) -> CmdResult<Vec<SearchHit>> {
        let db = self.db().clone();
        blocking(move || db.search(&query, network.as_deref(), target.as_deref(), limit.clamp(1, 500))).await?.map_err(err)
    }

    pub fn load_buffers(&self, conn: &str) -> CmdResult<Vec<SavedBuffer>> {
        self.db().buffers(conn).map_err(err)
    }

    pub fn save_buffers(&self, conn: &str, buffers: &[SavedBuffer]) -> CmdResult<()> {
        self.db().save_buffers(conn, buffers, |s| s.to_lowercase()).map_err(err)
    }

    pub async fn clear_logs(&self) -> CmdResult<()> {
        let db = self.db().clone();
        blocking(move || db.clear_logs()).await?.map_err(err)
    }

    pub async fn link_preview(&self, url: &str) -> CmdResult<Preview> {
        self.previews.preview(url).await.map_err(err)
    }

    /// A draft/metadata-2 avatar as a data: URL. Frontends never fetch remote
    /// images themselves, so the backend does (public addresses only).
    pub async fn avatar(&self, url: &str) -> CmdResult<String> {
        if !self.load_avatars.load(Ordering::Relaxed) {
            return Err("avatars are turned off".into());
        }
        self.previews.avatar(url).await.map_err(err)
    }

    pub fn generate_cert(&self, server: &str) -> CmdResult<CertInfo> {
        certs::generate(&self.config_dir.join("certs"), server).map_err(err)
    }

    pub fn cert_info(&self, path: &str) -> CmdResult<CertInfo> {
        certs::info(path).map_err(err)
    }

    pub async fn upload_file(&self, server: &str, url: &str, path: &str) -> CmdResult<String> {
        let cfg = self.server(server)?;
        let body = tokio::fs::read(path).await.map_err(err)?;
        let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        upload::upload(&cfg, url, &name, upload::guess_type(&name), body).await.map_err(err)
    }

    pub async fn upload_bytes(
        &self,
        server: &str,
        url: &str,
        name: &str,
        content_type: String,
        data: Vec<u8>,
    ) -> CmdResult<String> {
        let cfg = self.server(server)?;
        let ct = if content_type.is_empty() { upload::guess_type(name).to_string() } else { content_type };
        upload::upload(&cfg, url, name, &ct, data).await.map_err(err)
    }

    /// Run a command by its Tauri name with camelCase JSON arguments, as the
    /// Svelte frontend would invoke it. `upload_bytes` takes `data` as base64.
    pub async fn call(&self, cmd: &str, args: Value) -> CmdResult<Value> {
        let a = Args(args);
        match cmd {
            "get_config" => json(self.get_config()),
            "save_server" => json(self.save_server(a.get("server")?)?),
            "delete_server" => json(self.delete_server(&a.str("id")?)?),
            "save_settings" => json(self.save_settings(a.get("settings")?)?),
            "connect" => json(self.connect(&a.str("server")?)?),
            "disconnect" => {
                self.disconnect(&a.str("server")?, a.opt("reason")?);
                Ok(Value::Null)
            }
            "reconnect" => {
                self.reconnect(&a.str("server")?);
                Ok(Value::Null)
            }
            "reset" => {
                self.reset();
                Ok(Value::Null)
            }
            "send" => json(self.send(&a.str("conn")?, a.get("lines")?)?),
            "history" => {
                json(self.history(&a.str("conn")?, &a.str("kind")?, a.str("target")?, a.str("line")?).await?)
            }
            "local_history" => json(
                self.local_history(a.str("conn")?, a.str("target")?, a.opt("before")?, a.opt("after")?, a.get("limit")?)
                    .await?,
            ),
            "search" => json(self.search(a.str("query")?, a.opt("network")?, a.opt("target")?, a.get("limit")?).await?),
            "load_buffers" => json(self.load_buffers(&a.str("conn")?)?),
            "save_buffers" => json(self.save_buffers(&a.str("conn")?, &a.get::<Vec<SavedBuffer>>("buffers")?)?),
            "clear_logs" => json(self.clear_logs().await?),
            "link_preview" => json(self.link_preview(&a.str("url")?).await?),
            "avatar" => json(self.avatar(&a.str("url")?).await?),
            "generate_cert" => json(self.generate_cert(&a.str("server")?)?),
            "cert_info" => json(self.cert_info(&a.str("path")?)?),
            "upload_file" => json(self.upload_file(&a.str("server")?, &a.str("url")?, &a.str("path")?).await?),
            "upload_bytes" => {
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD.decode(a.str("data")?).map_err(err)?;
                let ct = a.opt::<String>("contentType")?.unwrap_or_default();
                json(self.upload_bytes(&a.str("server")?, &a.str("url")?, &a.str("name")?, ct, data).await?)
            }
            _ => Err(format!("unknown command {cmd}")),
        }
    }
}

fn json(v: impl Serialize) -> CmdResult<Value> {
    serde_json::to_value(v).map_err(err)
}

struct Args(Value);

impl Args {
    fn get<T: DeserializeOwned>(&self, key: &str) -> CmdResult<T> {
        let v = self.0.get(key).cloned().ok_or_else(|| format!("missing argument {key}"))?;
        serde_json::from_value(v).map_err(|e| format!("argument {key}: {e}"))
    }

    fn opt<T: DeserializeOwned>(&self, key: &str) -> CmdResult<Option<T>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("argument {key}: {e}")),
        }
    }

    fn str(&self, key: &str) -> CmdResult<String> {
        self.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn core() -> (Core, PathBuf) {
        let dir = std::env::temp_dir().join(format!("hearth-core-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        (Core::open(&dir, &dir, |_| {}).unwrap(), dir)
    }

    fn call(core: &Core, cmd: &str, args: Value) -> CmdResult<Value> {
        crate::runtime::handle().block_on(core.call(cmd, args))
    }

    #[test]
    fn dispatch_uses_the_tauri_names_and_arguments() {
        let (core, dir) = core();
        let settings = json!({"storeLogs": false, "theme": "dark"});
        call(&core, "save_settings", json!({ "settings": settings })).unwrap();
        let cfg = call(&core, "get_config", json!({})).unwrap();
        assert_eq!(cfg["settings"]["theme"], "dark");
        assert_eq!(cfg["servers"], json!([]));
        assert!(call(&core, "connect", json!({"server": "nope"})).unwrap_err().contains("unknown server"));
        assert!(call(&core, "send", json!({})).unwrap_err().contains("missing argument conn"));
        assert!(call(&core, "bogus", json!({})).unwrap_err().contains("unknown command"));
        let hits = call(&core, "search", json!({"query": "x", "network": null, "target": null, "limit": 5})).unwrap();
        assert_eq!(hits, json!([]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_buffers_round_trip() {
        let (core, dir) = core();
        let bufs = json!([{"name": "#Hearth", "kind": "channel", "key": ""}, {"name": "alice", "kind": "query", "key": ""}]);
        call(&core, "save_buffers", json!({"conn": "c", "buffers": bufs})).unwrap();
        core.db().flush();
        let back = call(&core, "load_buffers", json!({"conn": "c"})).unwrap();
        assert_eq!(back.as_array().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
