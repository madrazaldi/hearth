//! Shared harness for driving connection actors against a scripted IRC
//! server on localhost.
//!
//! Every wait here is bounded by wall-clock time rather than tokio's clock:
//! the lifecycle tests pause tokio time and step it by hand, so a tokio
//! timeout would never fire and a hung test would hang forever.

#![allow(dead_code)]

use hearth_core::config::ServerConfig;
use hearth_core::irc::client::{self, Cmd, Spec};
use hearth_core::irc::events::{Emitter, Event, Status};
use hearth_core::irc::manager::Manager;
use hearth_core::irc::message::Message;
use hearth_core::store::Db;
use std::future::Future;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

/// How long any single wait may take before the test fails.
pub const LIMIT: Duration = Duration::from_secs(10);

type Timers = Mutex<Vec<(Instant, oneshot::Sender<()>)>>;

/// One background thread serves every wall-clock timer, so waits don't cost
/// a thread each.
fn timers() -> &'static Timers {
    static TIMERS: OnceLock<Timers> = OnceLock::new();
    TIMERS.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_millis(2));
            let now = Instant::now();
            let mut list = timers().lock().unwrap();
            let mut i = 0;
            while i < list.len() {
                if list[i].1.is_closed() {
                    list.swap_remove(i);
                } else if list[i].0 <= now {
                    let _ = list.swap_remove(i).1.send(());
                } else {
                    i += 1;
                }
            }
        });
        Mutex::new(Vec::new())
    })
}

/// Sleep for wall-clock time, independent of tokio's (possibly paused) clock.
pub async fn real_sleep(d: Duration) {
    let (tx, rx) = oneshot::channel();
    timers().lock().unwrap().push((Instant::now() + d, tx));
    let _ = rx.await;
}

/// `f`'s output, or None if it takes longer than `d` of wall-clock time.
pub async fn within<T>(d: Duration, f: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        v = f => Some(v),
        _ = real_sleep(d) => None,
    }
}

pub async fn must<T>(what: &str, f: impl Future<Output = T>) -> T {
    within(LIMIT, f).await.unwrap_or_else(|| panic!("timed out waiting for {what}"))
}

/// Keeps a paused tokio clock from auto-advancing while held, so time moves
/// only through `tokio::time::advance` and socket I/O runs at its own pace.
/// Blocking tasks inhibit auto-advance on a current-thread runtime.
pub struct Hold(Option<std::sync::mpsc::Sender<()>>);

pub fn hold_clock() -> Hold {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    tokio::task::spawn_blocking(move || {
        let _ = rx.recv();
    });
    Hold(Some(tx))
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.take();
    }
}

/// Advance the paused clock in steps, letting the actor handle each timer
/// that fires along the way.
pub async fn advance(total: Duration, step: Duration) {
    let mut left = total;
    while !left.is_zero() {
        let d = step.min(left);
        tokio::time::advance(d).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        left -= d;
    }
}

pub fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

pub fn ms(m: u64) -> Duration {
    Duration::from_millis(m)
}

pub struct Peer {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    /// A partly read line survives a cancelled read here.
    buf: Vec<u8>,
}

impl Peer {
    pub async fn accept(listener: &TcpListener) -> Peer {
        must("the client to connect", Peer::accept_now(listener)).await
    }

    /// A connection arriving within `d`, if any.
    pub async fn try_accept(listener: &TcpListener, d: Duration) -> Option<Peer> {
        within(d, Peer::accept_now(listener)).await
    }

    async fn accept_now(listener: &TcpListener) -> Peer {
        let (sock, _) = listener.accept().await.unwrap();
        let (r, w) = sock.into_split();
        Peer { reader: BufReader::new(r), writer: w, buf: Vec::new() }
    }

    /// The next line, or None once the client hangs up.
    async fn next_now(&mut self) -> Option<String> {
        let n = self.reader.read_until(b'\n', &mut self.buf).await.unwrap_or(0);
        if n == 0 && !self.buf.ends_with(b"\n") {
            return None;
        }
        let line = String::from_utf8_lossy(&self.buf).trim_end().to_string();
        self.buf.clear();
        Some(line)
    }

    pub async fn next(&mut self) -> Option<String> {
        must("a client line", self.next_now()).await
    }

    pub async fn line(&mut self) -> String {
        self.next().await.expect("client hung up")
    }

    /// A line arriving within `d`, if any.
    pub async fn try_line(&mut self, d: Duration) -> Option<String> {
        within(d, self.next_now()).await.flatten()
    }

    /// Read lines until one starts with `prefix`, returning it.
    pub async fn expect(&mut self, prefix: &str) -> String {
        loop {
            let l = self.line().await;
            if l.starts_with(prefix) {
                return l;
            }
        }
    }

    /// Read until the client closes the connection, returning what it sent.
    pub async fn closed(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(l) = self.next().await {
            lines.push(l);
        }
        lines
    }

    pub async fn send(&mut self, line: &str) {
        self.writer.write_all(format!("{line}\r\n").as_bytes()).await.unwrap();
    }

    /// Assert the client sent nothing in reply to what came before: lines are
    /// handled in order, so the answer to a PING must be the next thing read.
    pub async fn assert_quiet(&mut self) {
        self.send("PING :sync").await;
        assert_eq!(self.line().await, "PONG sync");
    }

    pub async fn register(&mut self, caps: &str) {
        self.expect("CAP LS").await;
        self.send(&format!(":srv CAP * LS :{caps}")).await;
        if !caps.is_empty() {
            let req = self.expect("CAP REQ").await;
            let list = req.split_once(':').unwrap().1.to_string();
            self.send(&format!(":srv CAP * ACK :{list}")).await;
        }
        self.expect("CAP END").await;
        self.welcome().await;
    }

    pub async fn welcome(&mut self) {
        self.send(":srv 001 me :Welcome").await;
        self.send(":srv 005 me CHANTYPES=# CASEMAPPING=ascii :are supported").await;
        self.send(":srv 422 me :No MOTD").await;
    }
}

pub type Events = Arc<Mutex<Vec<Event>>>;

pub fn recorder() -> (Emitter, Events) {
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (Emitter::spawn(move |evs| sink.lock().unwrap().extend_from_slice(evs)), events)
}

pub fn server_config(listener: &TcpListener) -> ServerConfig {
    ServerConfig {
        id: "t".into(),
        host: "127.0.0.1".into(),
        port: listener.local_addr().unwrap().port(),
        tls: false,
        nick: "me".into(),
        ..Default::default()
    }
}

/// Wait for an emitted event matching `pred`.
pub async fn wait_event(events: &Events, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
    let find = || events.lock().unwrap().iter().find(|e| pred(e)).cloned();
    must(what, async {
        loop {
            if let Some(e) = find() {
                return e;
            }
            real_sleep(ms(5)).await;
        }
    })
    .await
}

pub async fn wait_message(events: &Events, pred: impl Fn(&Message) -> bool) -> Message {
    match wait_event(events, "a message event", |e| matches!(e, Event::Message { msg, .. } if pred(msg))).await {
        Event::Message { msg, .. } => msg,
        _ => unreachable!(),
    }
}

/// Wait until `n` events match `pred`, then return all of them.
pub async fn wait_count(events: &Events, n: usize, pred: impl Fn(&Event) -> bool) -> Vec<Event> {
    let all = || events.lock().unwrap().iter().filter(|e| pred(e)).cloned().collect::<Vec<_>>();
    must(&format!("{n} matching events"), async {
        loop {
            let found = all();
            if found.len() >= n {
                return found;
            }
            real_sleep(ms(5)).await;
        }
    })
    .await
}

pub fn count(events: &Events, pred: impl Fn(&Event) -> bool) -> usize {
    events.lock().unwrap().iter().filter(|e| pred(e)).count()
}

pub fn is_status(e: &Event, conn: &str, want: Status) -> bool {
    matches!(e, Event::Status { conn: c, status, .. } if c == conn && *status == want)
}

/// Connections spawned through a real `Manager`, as the app does.
pub struct Harness {
    pub mgr: Arc<Manager>,
    pub db: Arc<Db>,
    pub events: Events,
    pub listener: TcpListener,
}

impl Harness {
    pub async fn new() -> Harness {
        let db = Arc::new(Db::memory().unwrap());
        let (emitter, events) = recorder();
        let mgr = Manager::new(emitter, db.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Harness { mgr, db, events, listener }
    }

    pub fn server(&self) -> ServerConfig {
        server_config(&self.listener)
    }

    pub async fn event(&self, pred: impl Fn(&Message) -> bool) -> Message {
        wait_message(&self.events, pred).await
    }
}

/// One actor run directly on the test's runtime, so tests can pause and
/// step tokio's clock under it.
pub struct Direct {
    pub tx: mpsc::UnboundedSender<Cmd>,
    pub backlog: Arc<AtomicUsize>,
    pub db: Arc<Db>,
    pub events: Events,
    pub listener: TcpListener,
    pub task: tokio::task::JoinHandle<()>,
}

impl Direct {
    pub async fn start(configure: impl FnOnce(&mut ServerConfig), bind: Option<&str>) -> Direct {
        let db = Arc::new(Db::memory().unwrap());
        let (emitter, events) = recorder();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut server = server_config(&listener);
        configure(&mut server);
        let backlog = Arc::new(AtomicUsize::new(0));
        let spec = Spec {
            id: server.id.clone(),
            server,
            bind: bind.map(str::to_string),
            generation: 1,
            db: db.clone(),
            backlog: backlog.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(client::run(spec, rx, emitter, Weak::<Manager>::new()));
        Direct { tx, backlog, db, events, listener, task }
    }

    pub fn send(&self, lines: &[&str]) {
        self.tx.send(Cmd::Send(lines.iter().map(|l| l.to_string()).collect())).unwrap();
    }

    /// Stop the actor. Its QUIT may be waiting on the send pacing, so this
    /// needs a paused clock to step.
    pub async fn quit(self) {
        let _ = self.tx.send(Cmd::Quit(None));
        must("the actor to stop", async {
            while !self.task.is_finished() {
                advance(ms(500), ms(500)).await;
                real_sleep(ms(2)).await;
            }
        })
        .await;
    }

    /// Wait until the actor has handled a line with this command.
    pub async fn handled(&self, command: &str) -> Message {
        wait_message(&self.events, |m| m.command == command).await
    }
}

/// The retry delays announced in Disconnected events, in order.
pub fn retry_delays(events: &Events) -> Vec<Option<u64>> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            Event::Status { status: Status::Disconnected, retry_in, .. } => Some(*retry_in),
            _ => None,
        })
        .collect()
}

pub fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

pub fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}
