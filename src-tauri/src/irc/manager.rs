//! Owns every live connection. A server entry maps to one connection, or —
//! for a soju bouncer — a control connection plus one child per network.

use super::client::{self, Cmd, HistoryKind, Spec};
use super::events::{Emitter, Event};
use super::message::Message;
use crate::config::ServerConfig;
use crate::store::Db;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Lines a connection may have waiting for the socket before further sends
/// are refused. At the direct-connection pace of 2.5 lines/s this is minutes
/// of backlog, which only a flood (or a runaway paste) produces.
const MAX_BACKLOG: usize = 300;

struct Entry {
    tx: mpsc::UnboundedSender<Cmd>,
    generation: u64,
    server: String,
    backlog: Arc<AtomicUsize>,
}

pub struct Manager {
    conns: Mutex<HashMap<String, Entry>>,
    emitter: Emitter,
    db: Arc<Db>,
    generation: AtomicU64,
    me: std::sync::Weak<Manager>,
}

pub fn child_id(server: &str, netid: &str) -> String {
    format!("{server}/{netid}")
}

impl Manager {
    pub fn new(emitter: Emitter, db: Arc<Db>) -> Arc<Self> {
        Arc::new_cyclic(|me| Manager {
            conns: Mutex::new(HashMap::new()),
            emitter,
            db,
            generation: AtomicU64::new(1),
            me: me.clone(),
        })
    }

    fn spawn(&self, id: String, server: &ServerConfig, bind: Option<String>) {
        let Some(me) = self.me.upgrade() else { return };
        let mut conns = self.conns.lock().unwrap();
        if conns.contains_key(&id) {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        let backlog = Arc::new(AtomicUsize::new(0));
        conns.insert(id.clone(), Entry { tx, generation, server: server.id.clone(), backlog: backlog.clone() });
        drop(conns);

        self.emitter.emit(Event::ConnAdded {
            conn: id.clone(),
            server: server.id.clone(),
            parent: bind.as_ref().map(|_| server.id.clone()),
            netid: bind.clone(),
        });
        let spec = Spec { id, server: server.clone(), bind, generation, db: self.db.clone(), backlog };
        client::spawn(spec, rx, self.emitter.clone(), me);
    }

    pub fn connect(&self, server: &ServerConfig) {
        self.spawn(server.id.clone(), server, None);
    }

    pub fn ensure_child(&self, server: &ServerConfig, netid: &str) {
        self.spawn(child_id(&server.id, netid), server, Some(netid.to_string()));
    }

    pub fn remove_child(&self, server: &str, netid: &str) {
        self.quit(&child_id(server, netid), None);
    }

    fn quit(&self, id: &str, reason: Option<String>) {
        let removed = self.conns.lock().unwrap().remove(id);
        if let Some(e) = removed {
            let _ = e.tx.send(Cmd::Quit(reason));
            self.emitter.emit(Event::ConnRemoved { conn: id.to_string() });
        }
    }

    /// Disconnect a server and all of its bouncer children.
    pub fn disconnect(&self, server: &str, reason: Option<String>) {
        let ids: Vec<String> = self
            .conns
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| e.server == server)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.quit(&id, reason.clone());
        }
    }

    pub fn disconnect_all(&self) {
        let ids: Vec<String> = self.conns.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.quit(&id, None);
        }
    }

    pub fn reconnect(&self, server: &str) {
        for e in self.conns.lock().unwrap().values().filter(|e| e.server == server) {
            let _ = e.tx.send(Cmd::Reconnect);
        }
    }

    pub fn actor_finished(&self, id: &str, generation: u64) {
        let mut conns = self.conns.lock().unwrap();
        if conns.get(id).is_some_and(|e| e.generation == generation) {
            conns.remove(id);
            drop(conns);
            self.emitter.emit(Event::ConnRemoved { conn: id.to_string() });
        }
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    fn sender(&self, conn: &str) -> Result<mpsc::UnboundedSender<Cmd>> {
        self.conns
            .lock()
            .unwrap()
            .get(conn)
            .map(|e| e.tx.clone())
            .ok_or_else(|| anyhow!("no such connection"))
    }

    pub fn send(&self, conn: &str, lines: Vec<String>) -> Result<()> {
        let backlog = self.conns.lock().unwrap().get(conn).map(|e| e.backlog.load(Ordering::Relaxed)).unwrap_or(0);
        if backlog >= MAX_BACKLOG {
            return Err(anyhow!("too many lines waiting to be sent; try again in a moment"));
        }
        let lines = lines
            .into_iter()
            .map(|l| l.replace(['\r', '\n'], " "))
            .filter(|l| !l.trim().is_empty())
            .collect();
        self.sender(conn)?.send(Cmd::Send(lines)).map_err(|_| anyhow!("connection closed"))
    }

    pub async fn history(&self, conn: &str, kind: HistoryKind, target: String, line: String) -> Result<Vec<Message>> {
        let (reply, rx) = oneshot::channel();
        self.sender(conn)?
            .send(Cmd::History { kind, target, line, reply })
            .map_err(|_| anyhow!("connection closed"))?;
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(msgs))) => Ok(msgs),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("connection closed")),
            Err(_) => Err(anyhow!("history request timed out")),
        }
    }
}
