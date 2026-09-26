//! One IRC connection as an actor: it owns the socket, negotiates
//! capabilities and SASL, keeps the link alive, reconnects with backoff, and
//! forwards everything it reads to the frontend.

use super::events::{Emitter, Event, Status};
use super::manager::Manager;
use super::message::{casefold, Message, Source};
use super::scram::Scram;
use super::time::{format_ms, now_ms};
use super::transport::{self, Endpoint, LineWriter};
use crate::config::{SaslMechanism, ServerConfig};
use crate::store::{Db, LogEntry};
use anyhow::{anyhow, Result};
use base64::Engine;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

const WANTED_CAPS: &[&str] = &[
    "account-notify",
    "account-tag",
    "away-notify",
    "batch",
    "cap-notify",
    "chghost",
    "draft/chathistory",
    "draft/event-playback",
    "draft/extended-monitor",
    "draft/message-redaction",
    "draft/read-marker",
    "echo-message",
    "extended-join",
    "extended-monitor",
    "invite-notify",
    "labeled-response",
    "message-tags",
    "multi-prefix",
    "server-time",
    "setname",
    "soju.im/bouncer-networks",
    "soju.im/search",
    "userhost-in-names",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Messages,
    Targets,
    Search,
}

impl HistoryKind {
    fn batch_type(self) -> &'static str {
        match self {
            HistoryKind::Messages => "chathistory",
            HistoryKind::Targets => "draft/chathistory-targets",
            HistoryKind::Search => "soju.im/search",
        }
    }

    fn cap(self) -> &'static str {
        match self {
            HistoryKind::Search => "soju.im/search",
            _ => "draft/chathistory",
        }
    }
}

type Reply = oneshot::Sender<Result<Vec<Message>, String>>;

pub enum Cmd {
    Send(Vec<String>),
    History { kind: HistoryKind, target: String, line: String, reply: Reply },
    Quit(Option<String>),
    Reconnect,
}

pub struct Spec {
    pub id: String,
    pub server: ServerConfig,
    /// soju network id to bind to (child connections of a bouncer).
    pub bind: Option<String>,
    /// Distinguishes this actor from a later one reusing the same id.
    pub generation: u64,
    pub db: Arc<Db>,
    /// Lines queued for the socket but not yet written.
    pub backlog: Arc<AtomicUsize>,
}

enum End {
    Quit,
    Reconnect,
    Error(anyhow::Error, bool),
    /// The server's STS policy asks us to reconnect over TLS on this port.
    Upgrade(u16),
}

pub async fn run(spec: Spec, mut rx: mpsc::UnboundedReceiver<Cmd>, emitter: Emitter, manager: Weak<Manager>) {
    let mut attempt: u32 = 0;
    let mut upgrade: Option<u16> = None;
    loop {
        emitter.emit(Event::Status {
            conn: spec.id.clone(),
            status: Status::Connecting,
            error: None,
            retry_in: None,
        });
        let error = match session(&spec, upgrade, &mut rx, &emitter, &manager).await {
            End::Quit => break,
            End::Reconnect => {
                attempt = 0;
                None
            }
            End::Upgrade(port) => {
                log::info!("{}: STS policy requires TLS, reconnecting on port {port}", spec.id);
                upgrade = Some(port);
                attempt = 0;
                None
            }
            End::Error(e, was_registered) => {
                if was_registered {
                    attempt = 0;
                }
                Some(format!("{e:#}"))
            }
        };

        attempt = attempt.saturating_add(1);
        let delay = if error.is_none() { 0 } else { (2u64.pow(attempt.min(7))).min(120) };
        emitter.emit(Event::Status {
            conn: spec.id.clone(),
            status: Status::Disconnected,
            error: error.clone(),
            retry_in: error.as_ref().map(|_| delay),
        });

        let sleep = tokio::time::sleep(Duration::from_secs(delay));
        tokio::pin!(sleep);
        let quit = loop {
            tokio::select! {
                _ = &mut sleep => break false,
                cmd = rx.recv() => match cmd {
                    None | Some(Cmd::Quit(_)) => break true,
                    Some(Cmd::Reconnect) => break false,
                    Some(Cmd::History { reply, .. }) => { let _ = reply.send(Err("not connected".into())); }
                    Some(Cmd::Send(_)) => {}
                },
            }
        };
        if quit {
            break;
        }
    }
    if let Some(m) = manager.upgrade() {
        m.actor_finished(&spec.id, spec.generation);
    }
}

struct Pending {
    id: u64,
    kind: HistoryKind,
    /// Case-folded target, for matching unlabeled replies.
    target: String,
    label: Option<String>,
    /// A reply batch has been matched to this request.
    assigned: bool,
    messages: Vec<Message>,
    reply: Reply,
}

/// Where the contents of a batch go.
#[derive(Debug, Clone)]
enum Route {
    /// Collected as the reply to a pending request.
    Collect(u64),
    /// A labeled-response wrapper around the reply to a pending request.
    Wrap(u64),
    /// A labeled response to a line the frontend sent; its messages carry
    /// this label so the UI can route them.
    Label(String),
    Other,
}

enum Sasl {
    Plain,
    External,
    Scram(Box<Scram>),
}

struct State<'a> {
    spec: &'a Spec,
    emitter: &'a Emitter,
    manager: &'a Weak<Manager>,
    out: mpsc::UnboundedSender<String>,
    /// Whether this link is encrypted, and the port it uses (for STS).
    secure: bool,
    port: u16,
    nick: String,
    registered: bool,
    welcomed: bool,
    available: BTreeMap<String, String>,
    enabled: BTreeSet<String>,
    ls_buffer: Vec<String>,
    reqs_outstanding: usize,
    cap_ended: bool,
    isupport: BTreeMap<String, String>,
    batches: HashMap<String, Route>,
    pending: Vec<Pending>,
    next_id: u64,
    sasl: Option<Sasl>,
    sasl_step: u8,
    sasl_in: String,
    sasl_fell_back: bool,
    sts_upgrade: Option<u16>,
}

impl State<'_> {
    fn send(&self, line: impl Into<String>) {
        self.spec.backlog.fetch_add(1, Ordering::Relaxed);
        if self.out.send(line.into()).is_err() {
            self.spec.backlog.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn conn(&self) -> String {
        self.spec.id.clone()
    }

    fn casemapping(&self) -> &str {
        self.isupport.get("CASEMAPPING").map(String::as_str).unwrap_or("rfc1459")
    }

    fn fold(&self, s: &str) -> String {
        casefold(s, self.casemapping())
    }

    fn is_channel(&self, name: &str) -> bool {
        let types = self.isupport.get("CHANTYPES").map(String::as_str).unwrap_or("#&");
        name.chars().next().is_some_and(|c| types.contains(c))
    }

    fn is_me(&self, nick: &str) -> bool {
        self.fold(nick) == self.fold(&self.nick)
    }

    fn is_control(&self) -> bool {
        self.spec.bind.is_none() && self.enabled.contains("soju.im/bouncer-networks")
    }

    fn sasl_wanted(&self) -> bool {
        let s = &self.spec.server;
        match s.sasl {
            SaslMechanism::Plain => !s.sasl_password.is_empty(),
            SaslMechanism::External => !s.client_cert.trim().is_empty(),
            SaslMechanism::None => false,
        }
    }

    fn sasl_offered(&self) -> Vec<String> {
        self.available
            .get("sasl")
            .map(|v| v.split(',').filter(|m| !m.is_empty()).map(|m| m.to_ascii_uppercase()).collect())
            .unwrap_or_default()
    }

    fn request_caps(&mut self, offered: impl Iterator<Item = String>) {
        let mut want: Vec<String> = offered
            .filter(|c| {
                WANTED_CAPS.contains(&c.as_str())
                    || (c == "sasl" && self.sasl_wanted() && !self.registered)
                    || (c == "soju.im/bouncer-networks-notify" && self.spec.bind.is_none())
            })
            .filter(|c| !self.enabled.contains(c))
            .collect();
        want.sort();
        want.dedup();
        // Keep each REQ comfortably under the 512-byte line limit.
        let mut chunk = String::new();
        let mut chunks = Vec::new();
        for cap in want {
            if chunk.len() + cap.len() > 400 {
                chunks.push(std::mem::take(&mut chunk));
            }
            if !chunk.is_empty() {
                chunk.push(' ');
            }
            chunk.push_str(&cap);
        }
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        self.reqs_outstanding += chunks.len();
        for c in chunks {
            self.send(format!("CAP REQ :{c}"));
        }
    }

    fn maybe_end_cap(&mut self) {
        if self.cap_ended || self.registered || self.reqs_outstanding > 0 || self.sasl.is_some() {
            return;
        }
        if self.enabled.contains("sasl") && self.sasl_wanted() {
            self.start_sasl();
            return;
        }
        self.finish_cap();
    }

    fn start_sasl(&mut self) {
        let s = &self.spec.server;
        let offered = self.sasl_offered();
        let mech = match s.sasl {
            SaslMechanism::External => Sasl::External,
            _ if offered.iter().any(|m| m == "SCRAM-SHA-256") => {
                Sasl::Scram(Box::new(Scram::new(self.sasl_user(), &s.sasl_password)))
            }
            _ => Sasl::Plain,
        };
        self.begin_mechanism(mech);
    }

    fn begin_mechanism(&mut self, mech: Sasl) {
        let name = match mech {
            Sasl::Plain => "PLAIN",
            Sasl::External => "EXTERNAL",
            Sasl::Scram(_) => "SCRAM-SHA-256",
        };
        self.sasl = Some(mech);
        self.sasl_step = 0;
        self.sasl_in.clear();
        self.send(format!("AUTHENTICATE {name}"));
    }

    fn sasl_user(&self) -> &str {
        let s = &self.spec.server;
        if s.sasl_username.is_empty() { &s.nick } else { &s.sasl_username }
    }

    /// Send a SASL payload, base64-encoded and split into 400-byte chunks.
    fn send_sasl(&self, payload: &[u8]) {
        if payload.is_empty() {
            self.send("AUTHENTICATE +");
            return;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
        let bytes = encoded.as_bytes();
        for chunk in bytes.chunks(400) {
            self.send(format!("AUTHENTICATE {}", std::str::from_utf8(chunk).unwrap_or("")));
        }
        if bytes.len() % 400 == 0 {
            self.send("AUTHENTICATE +");
        }
    }

    fn handle_authenticate(&mut self, data: &str) {
        if data != "+" {
            self.sasl_in.push_str(data);
            if data.len() == 400 {
                return; // More chunks follow.
            }
        }
        let raw = std::mem::take(&mut self.sasl_in);
        let challenge = base64::engine::general_purpose::STANDARD.decode(raw).unwrap_or_default();
        let challenge = String::from_utf8_lossy(&challenge).to_string();
        let user = self.sasl_user().to_string();
        let password = self.spec.server.sasl_password.clone();
        let step = self.sasl_step;
        self.sasl_step += 1;
        let answer: Result<Option<Vec<u8>>, String> = match self.sasl.as_mut() {
            Some(Sasl::Plain) => Ok(Some(format!("{user}\0{user}\0{password}").into_bytes())),
            Some(Sasl::External) => Ok(Some(Vec::new())),
            Some(Sasl::Scram(scram)) => match step {
                0 => Ok(Some(scram.client_first().into_bytes())),
                1 => scram.client_final(&challenge).map(|s| Some(s.into_bytes())),
                _ => scram.verify(&challenge).map(|_| Some(Vec::new())),
            },
            None => Ok(None),
        };
        match answer {
            Ok(Some(payload)) => self.send_sasl(&payload),
            Ok(None) => {}
            Err(e) => {
                log::warn!("{}: SASL aborted: {e}", self.spec.id);
                self.forward(Message::new("FAIL", &["AUTHENTICATE", "SCRAM", &format!("SCRAM failed: {e}")]));
                self.send("AUTHENTICATE *");
            }
        }
    }

    fn finish_cap(&mut self) {
        if self.cap_ended {
            return;
        }
        self.cap_ended = true;
        if let Some(netid) = &self.spec.bind {
            self.send(format!("BOUNCER BIND {netid}"));
        }
        self.send("CAP END");
    }

    fn caps_event(&self) {
        self.emitter.emit(Event::Caps {
            conn: self.conn(),
            caps: self.enabled.iter().cloned().collect(),
        });
    }

    /// React to an advertised STS policy. Returns true when the connection
    /// must be dropped and re-established over TLS.
    fn check_sts(&mut self) -> bool {
        let Some(value) = self.available.get("sts") else { return false };
        let attrs: HashMap<&str, &str> = value.split(',').map(|kv| kv.split_once('=').unwrap_or((kv, ""))).collect();
        let s = &self.spec.server;
        if !self.secure {
            if s.websocket {
                return false;
            }
            if let Some(port) = attrs.get("port").and_then(|p| p.parse::<u16>().ok()) {
                self.sts_upgrade = Some(port);
                return true;
            }
        } else if !s.accept_invalid_certs {
            // Only persist a policy learned over a verified TLS connection.
            if let Some(duration) = attrs.get("duration").and_then(|d| d.parse::<i64>().ok()) {
                self.spec.db.set_sts_policy(s.host.trim(), self.port, duration);
            }
        }
        false
    }

    /// Returns Err to tear the session down.
    fn handle(&mut self, mut msg: Message) -> Result<()> {
        // Stamp arrival time once so the UI and the local log agree on it.
        if !msg.tags.contains_key("time") {
            msg.tags.insert("time".into(), format_ms(now_ms()));
        }
        // Route messages that belong to a batch.
        if let Some(batch) = msg.tag("batch").map(str::to_string) {
            match self.batches.get(&batch).cloned() {
                Some(Route::Collect(id)) | Some(Route::Wrap(id)) if msg.command != "BATCH" => {
                    if msg.command == "FAIL" {
                        self.complete(id, Err(msg.params.last().cloned().unwrap_or_default()));
                    } else if let Some(p) = self.pending.iter_mut().find(|p| p.id == id) {
                        p.messages.push(msg);
                    }
                    return Ok(());
                }
                Some(Route::Label(label)) => {
                    msg.tags.entry("label".into()).or_insert(label);
                }
                _ => {}
            }
        }

        match msg.command.as_str() {
            "PING" => self.send(Message::new("PONG", &msg.params.iter().map(String::as_str).collect::<Vec<_>>()).serialize()),
            "CAP" => {
                if self.handle_cap(&msg) {
                    return Err(anyhow!("upgrading to TLS as required by the server's STS policy"));
                }
            }
            "AUTHENTICATE" => self.handle_authenticate(msg.param(0)),
            "903" | "907" => {
                self.sasl = None;
                self.finish_cap();
            }
            "904" if matches!(self.sasl, Some(Sasl::Scram(_))) && !self.sasl_fell_back => {
                // Accounts without SCRAM credentials (common on Atheme) still
                // accept PLAIN; try it once before giving up.
                let offered = self.sasl_offered();
                if offered.is_empty() || offered.iter().any(|m| m == "PLAIN") {
                    self.sasl_fell_back = true;
                    self.begin_mechanism(Sasl::Plain);
                    return Ok(());
                }
                self.sasl = None;
                self.finish_cap();
            }
            "902" | "904" | "905" | "906" => {
                self.sasl = None;
                self.finish_cap();
            }
            "001" => {
                self.registered = true;
                self.nick = msg.param(0).to_string();
                self.emitter.emit(Event::Registered {
                    conn: self.conn(),
                    nick: self.nick.clone(),
                    caps: self.enabled.iter().cloned().collect(),
                });
                self.emitter.emit(Event::Status {
                    conn: self.conn(),
                    status: Status::Connected,
                    error: None,
                    retry_in: None,
                });
            }
            "005" => {
                let n = msg.params.len();
                for token in msg.params.iter().take(n.saturating_sub(1)).skip(1) {
                    if let Some(key) = token.strip_prefix('-') {
                        self.isupport.remove(key);
                    } else {
                        let (k, v) = token.split_once('=').unwrap_or((token, ""));
                        self.isupport.insert(k.to_string(), unescape_isupport(v));
                    }
                }
                self.emitter.emit(Event::Isupport {
                    conn: self.conn(),
                    isupport: self.isupport.clone(),
                });
            }
            "376" | "422" if !self.welcomed => {
                self.welcomed = true;
                self.on_welcome();
            }
            "433" | "432" if !self.registered => {
                let next = format!("{}_", self.nick);
                self.nick = next.chars().take(30).collect();
                self.send(format!("NICK {}", self.nick));
            }
            "NICK" => {
                if let Some(src) = &msg.source {
                    if self.is_me(&src.nick) {
                        self.nick = msg.param(0).to_string();
                    }
                }
            }
            "BATCH" => self.handle_batch(&msg),
            "ACK" => {
                if let Some(id) = msg.tag("label").and_then(|l| self.pending_by_label(l)) {
                    self.complete(id, Ok(Vec::new()));
                    return Ok(());
                }
            }
            "FAIL" if matches!(msg.param(0), "CHATHISTORY" | "SEARCH") => {
                if let Some(id) = self.pending_for_fail(&msg) {
                    self.complete(id, Err(msg.params.last().cloned().unwrap_or_default()));
                }
            }
            "BOUNCER" if msg.param(0) == "NETWORK" && self.is_control() => {
                if let Some(m) = self.manager.upgrade() {
                    let netid = msg.param(1).to_string();
                    if msg.param(2) == "*" {
                        m.remove_child(&self.spec.id, &netid);
                    } else {
                        m.ensure_child(&self.spec.server, &netid);
                    }
                }
            }
            "ERROR" => {
                let reason = msg.param(0).to_string();
                self.forward(msg);
                return Err(anyhow!("{}", if reason.is_empty() { "server closed the connection".into() } else { reason }));
            }
            _ => {}
        }
        self.log_live(&msg);
        self.forward(msg);
        Ok(())
    }

    fn forward(&self, msg: Message) {
        self.emitter.emit(Event::Message { conn: self.conn(), msg });
    }

    /// Returns true when an STS upgrade is required.
    fn handle_cap(&mut self, msg: &Message) -> bool {
        let sub = msg.param(1).to_ascii_uppercase();
        let last = msg.params.last().cloned().unwrap_or_default();
        match sub.as_str() {
            "LS" => {
                self.ls_buffer.push(last);
                // A "*" before the list means more lines follow.
                if msg.params.len() >= 4 && msg.param(2) == "*" {
                    return false;
                }
                let all = std::mem::take(&mut self.ls_buffer).join(" ");
                for tok in all.split_whitespace() {
                    let (k, v) = tok.split_once('=').unwrap_or((tok, ""));
                    self.available.insert(k.to_string(), v.to_string());
                }
                if self.check_sts() {
                    return true;
                }
                let offered: Vec<String> = self.available.keys().cloned().collect();
                self.request_caps(offered.into_iter());
                self.maybe_end_cap();
            }
            "ACK" => {
                for c in last.split_whitespace() {
                    match c.strip_prefix('-') {
                        Some(removed) => {
                            self.enabled.remove(removed);
                        }
                        None => {
                            self.enabled.insert(c.to_string());
                        }
                    }
                }
                self.reqs_outstanding = self.reqs_outstanding.saturating_sub(1);
                self.maybe_end_cap();
                self.caps_event();
            }
            "NAK" => {
                self.reqs_outstanding = self.reqs_outstanding.saturating_sub(1);
                self.maybe_end_cap();
            }
            "NEW" => {
                let mut offered = Vec::new();
                for tok in last.split_whitespace() {
                    let (k, v) = tok.split_once('=').unwrap_or((tok, ""));
                    self.available.insert(k.to_string(), v.to_string());
                    offered.push(k.to_string());
                }
                if offered.iter().any(|c| c == "sts") && self.check_sts() {
                    return true;
                }
                self.request_caps(offered.into_iter());
            }
            "DEL" => {
                for c in last.split_whitespace() {
                    self.available.remove(c);
                    self.enabled.remove(c);
                }
                self.caps_event();
            }
            _ => {}
        }
        false
    }

    fn pending_by_label(&self, label: &str) -> Option<u64> {
        self.pending.iter().find(|p| p.label.as_deref() == Some(label)).map(|p| p.id)
    }

    /// The oldest unlabeled request of `kind` still waiting for its batch,
    /// preferring one for `target`.
    fn unassigned(&self, kind: HistoryKind, target: Option<&str>) -> Option<u64> {
        let open = || self.pending.iter().filter(|p| !p.assigned && p.label.is_none() && p.kind == kind);
        target
            .and_then(|t| open().find(|p| p.target == t))
            .or_else(|| open().next())
            .map(|p| p.id)
    }

    fn pending_for_fail(&self, msg: &Message) -> Option<u64> {
        if let Some(id) = msg.tag("label").and_then(|l| self.pending_by_label(l)) {
            return Some(id);
        }
        let kind = match (msg.param(0), msg.param(2)) {
            ("SEARCH", _) => HistoryKind::Search,
            (_, "TARGETS") => HistoryKind::Targets,
            _ => HistoryKind::Messages,
        };
        // FAIL CHATHISTORY <code> <subcommand> [<target>] :<description>
        let target = (msg.params.len() > 4).then(|| self.fold(msg.param(3)));
        self.unassigned(kind, target.as_deref())
    }

    fn complete(&mut self, id: u64, result: Result<Vec<Message>, String>) {
        let Some(i) = self.pending.iter().position(|p| p.id == id) else { return };
        let p = self.pending.remove(i);
        if let (Ok(msgs), HistoryKind::Messages) = (&result, p.kind) {
            // Backfill the local log; only messages with an id can be deduplicated.
            for m in msgs.iter().filter(|m| m.tag("msgid").is_some() && loggable(m)) {
                self.spec.db.log(LogEntry { network: self.conn(), target: p.target.clone(), msg: m.clone() });
            }
        }
        let _ = p.reply.send(result);
    }

    fn handle_batch(&mut self, msg: &Message) {
        let reference = msg.param(0);
        if let Some(r) = reference.strip_prefix('+') {
            let kind = msg.param(1);
            let parent = msg.tag("batch").and_then(|b| self.batches.get(b)).cloned();
            let own = msg.tag("label").and_then(|l| self.pending_by_label(l));
            let route = if kind == "labeled-response" {
                match (own, msg.tag("label")) {
                    (Some(id), _) => Route::Wrap(id),
                    (None, Some(label)) => Route::Label(label.to_string()),
                    _ => Route::Other,
                }
            } else {
                let hkind = [HistoryKind::Messages, HistoryKind::Targets, HistoryKind::Search]
                    .into_iter()
                    .find(|k| k.batch_type() == kind);
                let id = hkind.and_then(|hk| {
                    own.or(match parent {
                        Some(Route::Wrap(id)) => Some(id),
                        _ => None,
                    })
                    .or_else(|| {
                        let target = (hk == HistoryKind::Messages).then(|| self.fold(msg.param(2)));
                        self.unassigned(hk, target.as_deref())
                    })
                });
                match (id, parent) {
                    (Some(id), _) => {
                        if let Some(p) = self.pending.iter_mut().find(|p| p.id == id) {
                            p.assigned = true;
                        }
                        Route::Collect(id)
                    }
                    (None, Some(Route::Label(l))) => Route::Label(l),
                    _ => Route::Other,
                }
            };
            self.batches.insert(r.to_string(), route);
        } else if let Some(r) = reference.strip_prefix('-') {
            match self.batches.remove(r) {
                Some(Route::Collect(id)) | Some(Route::Wrap(id)) => {
                    let msgs = self
                        .pending
                        .iter_mut()
                        .find(|p| p.id == id)
                        .map(|p| std::mem::take(&mut p.messages));
                    if let Some(msgs) = msgs {
                        self.complete(id, Ok(msgs));
                    }
                }
                _ => {}
            }
        }
    }

    /// The buffer a message belongs in, case-folded, if it should be logged.
    fn log_target(&self, msg: &Message) -> Option<String> {
        let src = msg.source.as_ref();
        let target = msg.param(0);
        match msg.command.as_str() {
            "PRIVMSG" | "NOTICE" | "TAGMSG" => {
                let src = src?;
                // Server notices have no user part and usually a dotted name.
                if src.user.is_none() && (src.nick.contains('.') || !self.registered) {
                    return None;
                }
                let statusmsg = self.isupport.get("STATUSMSG").map(String::as_str).unwrap_or("");
                let mut t = target;
                while t.len() > 1 && statusmsg.contains(&t[..1]) && self.is_channel(&t[1..]) {
                    t = &t[1..];
                }
                if t.is_empty() || t == "*" {
                    return None;
                }
                let peer = if self.is_channel(t) || self.is_me(&src.nick) { t } else { &src.nick };
                Some(self.fold(peer))
            }
            "JOIN" | "PART" | "KICK" | "TOPIC" => Some(self.fold(target)),
            "MODE" if self.is_channel(target) => Some(self.fold(target)),
            _ => None,
        }
    }

    fn log_live(&self, msg: &Message) {
        if self.is_control() {
            return;
        }
        if msg.command == "REDACT" {
            self.spec.db.redact(&self.spec.id, msg.param(1));
            return;
        }
        if !self.spec.db.logging() || !loggable(msg) {
            return;
        }
        if let Some(target) = self.log_target(msg) {
            self.spec.db.log(LogEntry { network: self.conn(), target, msg: msg.clone() });
        }
    }

    /// Without echo-message the server never reflects our own messages, so
    /// log them as they are sent.
    fn log_outgoing(&self, line: &str) {
        if self.enabled.contains("echo-message") || self.is_control() || !self.spec.db.logging() {
            return;
        }
        let Some(mut msg) = Message::parse(line) else { return };
        if !matches!(msg.command.as_str(), "PRIVMSG" | "NOTICE") || !loggable(&msg) {
            return;
        }
        msg.source = Some(Source { nick: self.nick.clone(), user: Some(String::new()), host: None });
        msg.tags.insert("time".into(), format_ms(now_ms()));
        let target = self.fold(msg.param(0));
        self.spec.db.log(LogEntry { network: self.conn(), target, msg });
    }

    fn on_welcome(&mut self) {
        if self.is_control() {
            return;
        }
        // Bouncers restore channels themselves; only join when talking to a network directly.
        let s = &self.spec.server;
        if self.spec.bind.is_none() {
            let chans: Vec<&String> = s.autojoin.iter().filter(|c| !c.trim().is_empty()).collect();
            for group in chans.chunks(10) {
                let names: Vec<&str> = group.iter().map(|c| c.trim()).collect();
                self.send(format!("JOIN {}", names.join(",")));
            }
            for line in s.perform.iter().filter(|l| !l.trim().is_empty()) {
                let line = line.trim().trim_start_matches('/');
                self.send(line.replace("$nick", &self.nick));
            }
        }
    }
}

/// Whether a message is worth keeping in the local log.
fn loggable(msg: &Message) -> bool {
    match msg.command.as_str() {
        "PRIVMSG" | "NOTICE" => {
            let body = msg.param(1);
            !body.starts_with('\x01') || body.starts_with("\x01ACTION")
        }
        "TAGMSG" => msg.tag("+draft/react").is_some(),
        "JOIN" | "PART" | "KICK" | "TOPIC" | "MODE" => true,
        _ => false,
    }
}

fn unescape_isupport(v: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        let hex = rest.get(i + 2..i + 4).unwrap_or("");
        match u8::from_str_radix(hex, 16) {
            Ok(b) => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            Err(_) => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

async fn session(
    spec: &Spec,
    upgrade: Option<u16>,
    rx: &mut mpsc::UnboundedReceiver<Cmd>,
    emitter: &Emitter,
    manager: &Weak<Manager>,
) -> End {
    let s = &spec.server;
    let host = s.host.trim().to_string();
    // A remembered STS policy upgrades plaintext connections before they start.
    let policy = if !s.tls && !s.websocket { spec.db.sts_policy(&host) } else { None };
    let (tls, port) = match (upgrade, policy) {
        (Some(port), _) => (true, port),
        (None, Some(p)) => (true, p.port),
        _ => (s.tls, s.port),
    };
    let client_cert = s.client_cert.trim();
    let ep = Endpoint {
        host,
        port,
        tls,
        accept_invalid_certs: s.accept_invalid_certs,
        websocket_path: s.websocket.then(|| s.websocket_path.clone()),
        client_cert: (tls && !client_cert.is_empty()).then(|| client_cert.to_string()),
    };
    let (mut reader, writer) = match transport::connect(&ep).await {
        Ok(rw) => rw,
        Err(e) => return End::Error(e, false),
    };
    emitter.emit(Event::Status {
        conn: spec.id.clone(),
        status: Status::Registering,
        error: None,
        retry_in: None,
    });

    let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
    let throttle = spec.bind.is_none() && !s.websocket;
    spec.backlog.store(0, Ordering::Relaxed);
    let mut writer_task = tokio::spawn(write_loop(writer, out_rx, throttle, spec.backlog.clone()));

    let nick = if s.nick.trim().is_empty() { "hearth".to_string() } else { s.nick.trim().to_string() };
    let mut st = State {
        spec,
        emitter,
        manager,
        out: out_tx,
        secure: tls,
        port,
        nick: nick.clone(),
        registered: false,
        welcomed: false,
        available: BTreeMap::new(),
        enabled: BTreeSet::new(),
        ls_buffer: Vec::new(),
        reqs_outstanding: 0,
        cap_ended: false,
        isupport: BTreeMap::new(),
        batches: HashMap::new(),
        pending: Vec::new(),
        next_id: 1,
        sasl: None,
        sasl_step: 0,
        sasl_in: String::new(),
        sasl_fell_back: false,
        sts_upgrade: None,
    };

    st.send("CAP LS 302");
    if !s.password.is_empty() {
        st.send(format!("PASS {}", s.password));
    }
    let username = if s.username.trim().is_empty() { nick.clone() } else { s.username.trim().to_string() };
    let realname = if s.realname.trim().is_empty() { nick.clone() } else { s.realname.trim().to_string() };
    st.send(format!("NICK {nick}"));
    st.send(Message::new("USER", &[&username, "0", "*", &realname]).serialize());

    let started = Instant::now();
    let mut last_rx = Instant::now();
    let mut pinged = false;
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    let mut writer_finished = false;

    let end = loop {
        tokio::select! {
            line = reader.next_line() => match line {
                Ok(Some(line)) => {
                    last_rx = Instant::now();
                    pinged = false;
                    let Some(msg) = Message::parse(&line) else { continue };
                    if let Err(e) = st.handle(msg) {
                        if let Some(port) = st.sts_upgrade {
                            break End::Upgrade(port);
                        }
                        break End::Error(e, st.registered);
                    }
                }
                Ok(None) => break End::Error(anyhow!("connection closed by server"), st.registered),
                Err(e) => break End::Error(e, st.registered),
            },
            cmd = rx.recv() => match cmd {
                None => break End::Quit,
                Some(Cmd::Send(lines)) => for l in lines {
                    st.log_outgoing(&l);
                    st.send(l);
                },
                Some(Cmd::History { kind, target, line, reply }) => {
                    if !st.enabled.contains(kind.cap()) {
                        let _ = reply.send(Err("server does not support this request".into()));
                    } else {
                        let id = st.next_id;
                        st.next_id += 1;
                        let label = st.enabled.contains("labeled-response").then(|| format!("h{id}"));
                        let line = match &label {
                            Some(l) => format!("@label={l} {line}"),
                            None => line,
                        };
                        let target = st.fold(&target);
                        st.pending.push(Pending { id, kind, target, label, assigned: false, messages: Vec::new(), reply });
                        st.send(line);
                    }
                }
                Some(Cmd::Quit(reason)) => {
                    st.send(format!("QUIT :{}", reason.unwrap_or_else(|| "Hearth".into())));
                    break End::Quit;
                }
                Some(Cmd::Reconnect) => {
                    st.send("QUIT :Reconnecting");
                    break End::Reconnect;
                }
            },
            _ = tick.tick() => {
                let idle = last_rx.elapsed();
                if !st.registered && started.elapsed() > Duration::from_secs(60) {
                    break End::Error(anyhow!("registration timed out"), false);
                }
                if pinged && idle > Duration::from_secs(150) {
                    break End::Error(anyhow!("ping timeout"), st.registered);
                }
                if !pinged && idle > Duration::from_secs(90) {
                    pinged = true;
                    st.send("PING :hearth");
                }
            },
            res = &mut writer_task => {
                writer_finished = true;
                let err = match res { Ok(Err(e)) => e, _ => anyhow!("writer stopped") };
                break End::Error(err, st.registered);
            }
        }
    };

    for p in st.pending.drain(..) {
        let _ = p.reply.send(Err("disconnected".into()));
    }
    // Let the writer flush (e.g. QUIT) before the socket drops.
    drop(st);
    if !writer_finished {
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut writer_task).await;
        writer_task.abort();
    }
    end
}

async fn write_loop(
    mut writer: LineWriter,
    mut rx: mpsc::UnboundedReceiver<String>,
    throttle: bool,
    backlog: Arc<AtomicUsize>,
) -> Result<()> {
    // Token bucket: bursts of 8, then one line every 400ms. Bouncers do their own pacing.
    let mut tokens: f64 = 8.0;
    let mut last = Instant::now();
    while let Some(line) = rx.recv().await {
        if throttle {
            let now = Instant::now();
            tokens = (tokens + now.duration_since(last).as_secs_f64() / 0.4).min(8.0);
            last = now;
            if tokens < 1.0 {
                let wait = (1.0 - tokens) * 0.4;
                tokio::time::sleep(Duration::from_secs_f64(wait)).await;
                tokens = 1.0;
                last = Instant::now();
            }
            tokens -= 1.0;
        }
        let quitting = line.starts_with("QUIT");
        let sent = writer.send_line(&line).await;
        backlog.fetch_sub(1, Ordering::Relaxed);
        sent?;
        if quitting {
            break;
        }
    }
    writer.close().await;
    Ok(())
}

pub fn spawn(spec: Spec, rx: mpsc::UnboundedReceiver<Cmd>, emitter: Emitter, manager: Arc<Manager>) {
    let weak = Arc::downgrade(&manager);
    tauri::async_runtime::spawn(run(spec, rx, emitter, weak));
}
