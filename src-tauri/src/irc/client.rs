//! One IRC connection as an actor: it owns the socket, negotiates
//! capabilities and SASL, keeps the link alive, reconnects with backoff, and
//! forwards everything it reads to the frontend.

use super::events::{Emitter, Event, Status};
use super::manager::Manager;
use super::message::{casefold, Message};
use super::transport::{self, Endpoint, LineWriter};
use crate::config::{SaslMechanism, ServerConfig};
use anyhow::{anyhow, Result};
use base64::Engine;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    "draft/message-redaction",
    "draft/read-marker",
    "echo-message",
    "extended-join",
    "invite-notify",
    "message-tags",
    "multi-prefix",
    "server-time",
    "setname",
    "soju.im/bouncer-networks",
    "userhost-in-names",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Messages,
    Targets,
}

pub enum Cmd {
    Send(Vec<String>),
    History {
        kind: HistoryKind,
        target: String,
        line: String,
        reply: oneshot::Sender<Result<Vec<Message>, String>>,
    },
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
}

enum End {
    Quit,
    Reconnect,
    Error(anyhow::Error, bool),
}

pub async fn run(spec: Spec, mut rx: mpsc::UnboundedReceiver<Cmd>, emitter: Emitter, manager: Weak<Manager>) {
    let mut attempt: u32 = 0;
    loop {
        emitter.emit(Event::Status {
            conn: spec.id.clone(),
            status: Status::Connecting,
            error: None,
            retry_in: None,
        });
        let error = match session(&spec, &mut rx, &emitter, &manager).await {
            End::Quit => break,
            End::Reconnect => {
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
    kind: HistoryKind,
    target: String,
    batch: Option<String>,
    messages: Vec<Message>,
    reply: oneshot::Sender<Result<Vec<Message>, String>>,
}

struct State<'a> {
    spec: &'a Spec,
    emitter: &'a Emitter,
    manager: &'a Weak<Manager>,
    out: mpsc::UnboundedSender<String>,
    nick: String,
    registered: bool,
    welcomed: bool,
    available: BTreeMap<String, String>,
    enabled: BTreeSet<String>,
    ls_buffer: Vec<String>,
    reqs_outstanding: usize,
    cap_ended: bool,
    isupport: BTreeMap<String, String>,
    batches: HashMap<String, Option<usize>>,
    pending: Vec<Pending>,
}

impl State<'_> {
    fn send(&self, line: impl Into<String>) {
        let _ = self.out.send(line.into());
    }

    fn conn(&self) -> String {
        self.spec.id.clone()
    }

    fn casemapping(&self) -> &str {
        self.isupport.get("CASEMAPPING").map(String::as_str).unwrap_or("rfc1459")
    }

    fn is_control(&self) -> bool {
        self.spec.bind.is_none() && self.enabled.contains("soju.im/bouncer-networks")
    }

    fn sasl_wanted(&self) -> bool {
        let s = &self.spec.server;
        s.sasl == SaslMechanism::Plain && !s.sasl_password.is_empty()
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
        if self.cap_ended || self.registered || self.reqs_outstanding > 0 {
            return;
        }
        if self.enabled.contains("sasl") && self.sasl_wanted() {
            self.send("AUTHENTICATE PLAIN");
            return;
        }
        self.finish_cap();
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

    /// Returns Err to tear the session down.
    fn handle(&mut self, msg: Message) -> Result<()> {
        // Route messages that belong to a requested history batch.
        if let Some(batch) = msg.tag("batch") {
            if let Some(Some(idx)) = self.batches.get(batch) {
                let idx = *idx;
                if msg.command != "BATCH" {
                    if let Some(p) = self.pending.get_mut(idx) {
                        p.messages.push(msg);
                    }
                    return Ok(());
                }
            }
        }

        match msg.command.as_str() {
            "PING" => self.send(Message::new("PONG", &msg.params.iter().map(String::as_str).collect::<Vec<_>>()).serialize()),
            "CAP" => self.handle_cap(&msg),
            "AUTHENTICATE" if msg.param(0) == "+" => {
                let s = &self.spec.server;
                let user = if s.sasl_username.is_empty() { &s.nick } else { &s.sasl_username };
                let payload = format!("{user}\0{user}\0{}", s.sasl_password);
                let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
                let bytes = encoded.as_bytes();
                for chunk in bytes.chunks(400) {
                    self.send(format!("AUTHENTICATE {}", std::str::from_utf8(chunk).unwrap_or("")));
                }
                if bytes.len() % 400 == 0 {
                    self.send("AUTHENTICATE +");
                }
            }
            "903" | "907" => self.finish_cap(),
            "902" | "904" | "905" | "906" | "908" => {
                if msg.command != "908" {
                    self.finish_cap();
                }
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
                    let cm = self.casemapping().to_string();
                    if casefold(&src.nick, &cm) == casefold(&self.nick, &cm) {
                        self.nick = msg.param(0).to_string();
                    }
                }
            }
            "BATCH" => self.handle_batch(&msg),
            "FAIL" if msg.param(0) == "CHATHISTORY" => {
                if let Some(i) = self.pending.iter().position(|p| p.batch.is_none()) {
                    let p = self.pending.remove(i);
                    self.reindex_batches(i);
                    let _ = p.reply.send(Err(msg.params.last().cloned().unwrap_or_default()));
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
        self.forward(msg);
        Ok(())
    }

    fn forward(&self, msg: Message) {
        self.emitter.emit(Event::Message { conn: self.conn(), msg });
    }

    fn handle_cap(&mut self, msg: &Message) {
        let sub = msg.param(1).to_ascii_uppercase();
        let last = msg.params.last().cloned().unwrap_or_default();
        match sub.as_str() {
            "LS" => {
                self.ls_buffer.push(last);
                // A "*" before the list means more lines follow.
                if msg.params.len() >= 4 && msg.param(2) == "*" {
                    return;
                }
                let all = std::mem::take(&mut self.ls_buffer).join(" ");
                for tok in all.split_whitespace() {
                    let (k, v) = tok.split_once('=').unwrap_or((tok, ""));
                    self.available.insert(k.to_string(), v.to_string());
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
    }

    fn handle_batch(&mut self, msg: &Message) {
        let reference = msg.param(0);
        if let Some(r) = reference.strip_prefix('+') {
            let kind = msg.param(1);
            let cm = self.casemapping().to_string();
            let slot = match kind {
                "chathistory" => {
                    let target = casefold(msg.param(2), &cm);
                    self.pending
                        .iter()
                        .position(|p| p.batch.is_none() && p.kind == HistoryKind::Messages && p.target == target)
                }
                "draft/chathistory-targets" => self
                    .pending
                    .iter()
                    .position(|p| p.batch.is_none() && p.kind == HistoryKind::Targets),
                _ => None,
            };
            if let Some(i) = slot {
                self.pending[i].batch = Some(r.to_string());
            }
            self.batches.insert(r.to_string(), slot);
        } else if let Some(r) = reference.strip_prefix('-') {
            if let Some(Some(i)) = self.batches.remove(r) {
                let p = self.pending.remove(i);
                self.reindex_batches(i);
                let _ = p.reply.send(Ok(p.messages));
            }
        }
    }

    fn reindex_batches(&mut self, removed: usize) {
        for slot in self.batches.values_mut().flatten() {
            if *slot > removed {
                *slot -= 1;
            }
        }
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

async fn session(spec: &Spec, rx: &mut mpsc::UnboundedReceiver<Cmd>, emitter: &Emitter, manager: &Weak<Manager>) -> End {
    let s = &spec.server;
    let ep = Endpoint {
        host: s.host.trim().to_string(),
        port: s.port,
        tls: s.tls,
        accept_invalid_certs: s.accept_invalid_certs,
        websocket_path: s.websocket.then(|| s.websocket_path.clone()),
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
    let mut writer_task = tokio::spawn(write_loop(writer, out_rx, throttle));

    let nick = if s.nick.trim().is_empty() { "hearth".to_string() } else { s.nick.trim().to_string() };
    let mut st = State {
        spec,
        emitter,
        manager,
        out: out_tx,
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
                        break End::Error(e, st.registered);
                    }
                }
                Ok(None) => break End::Error(anyhow!("connection closed by server"), st.registered),
                Err(e) => break End::Error(e, st.registered),
            },
            cmd = rx.recv() => match cmd {
                None => break End::Quit,
                Some(Cmd::Send(lines)) => for l in lines { st.send(l) },
                Some(Cmd::History { kind, target, line, reply }) => {
                    if !st.enabled.contains("draft/chathistory") {
                        let _ = reply.send(Err("server does not support chat history".into()));
                    } else {
                        let target = casefold(&target, st.casemapping());
                        st.pending.push(Pending { kind, target, batch: None, messages: Vec::new(), reply });
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

async fn write_loop(mut writer: LineWriter, mut rx: mpsc::UnboundedReceiver<String>, throttle: bool) -> Result<()> {
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
        writer.send_line(&line).await?;
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
