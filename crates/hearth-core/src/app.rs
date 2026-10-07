//! The command API the frontend uses. Tauri wraps each method in a
//! `#[tauri::command]`.

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

    #[test]
    fn settings_and_config_round_trip() {
        let (core, dir) = core();
        core.save_settings(json!({"storeLogs": false, "theme": "dark"})).unwrap();
        let cfg = core.get_config();
        assert_eq!(cfg.settings["theme"], "dark");
        assert!(cfg.servers.is_empty());
        assert!(core.connect("nope").unwrap_err().contains("unknown server"));
        assert!(core.send("nope", vec![]).is_err());
        let hits = crate::runtime::handle().block_on(core.search("x".into(), None, None, 5)).unwrap();
        assert!(hits.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_buffers_round_trip() {
        let (core, dir) = core();
        let bufs: Vec<SavedBuffer> =
            serde_json::from_value(json!([{"name": "#Hearth", "kind": "channel", "key": ""}, {"name": "alice", "kind": "query", "key": ""}]))
                .unwrap();
        core.save_buffers("c", &bufs).unwrap();
        core.db().flush();
        let back = core.load_buffers("c").unwrap();
        assert_eq!(back.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
