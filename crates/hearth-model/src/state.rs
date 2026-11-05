//! `App`: everything the UI shows and every action it can take.
//!
//! The model is a synchronous state machine. The host feeds it IRC events
//! ([`App::handle_events`]), answers to its requests ([`App::resolve`]) and
//! the passage of time ([`App::tick`]), and drains the side effects it queued
//! ([`App::take_effects`]). Nothing here blocks or spawns.

use crate::buffer::{BufId, Buffer, BufferKind, HistoryState};
use crate::conn::Conn;
use crate::effect::{Effect, Reply, ReqId, Secret};
use crate::env::{Env, Timer};
use crate::format::{mentions, strip_formatting};
use crate::history::{AroundStage, GapFrom, HistMode};
use crate::ignore::{is_ignored, normalize_mask};
use crate::irc::{escape_tag, split_message};
use crate::media::ByteLru;
use crate::multiline::{join_parts, multiline_batches, parse_multiline_cap};
use crate::panels::{BrowserState, SettingsView, Switcher};
use crate::search_panel::SearchPanel;
use crate::time::Zone;
use crate::types::{ConnStatus, IrcEvent, Line, LineType, NetworkHighlights, NotifyLevel, Preview, ServerConfig, Settings, Source};
use crate::welcome::WelcomeFlow;
use hearth_core::config::Config;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToastActionKind {
    /// Use the account from `/register` for SASL from now on.
    SaveLogin { conn: String, account: String, password: Secret<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToastAction {
    pub label: String,
    pub kind: ToastActionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub kind: ToastKind,
    pub action: Option<ToastAction>,
    pub expires_at: i64,
}

/// What a request in flight is for. Short-lived and kept in a map, so the
/// size of the biggest variant doesn't matter.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Pending {
    History {
        buf: BufId,
        mode: HistMode,
        limit: usize,
    },
    Around {
        buf: BufId,
        time: i64,
        msgid: Option<String>,
        stage: AroundStage,
    },
    Gap {
        buf: BufId,
        gap: crate::buffer::Gap,
        from: GapFrom,
        remote: bool,
        limit: usize,
    },
    Targets {
        conn: String,
    },
    Restore {
        conn: String,
    },
    /// `login` is the account and network to name in the "will sign in" toast.
    SaveServer {
        connect_now: bool,
        login: Option<(String, String)>,
    },
    Search(crate::search::SearchStep),
    Preview {
        url: String,
    },
    Avatar {
        url: String,
    },
    ClearLogs,
    Upload {
        buf: BufId,
    },
    /// Answered for the UI to pick up with `App::take_result`.
    Ui,
}

/// A link preview as the UI sees it.
#[derive(Debug, Clone)]
pub enum PreviewState {
    Loading,
    /// The page has no preview, or fetching failed.
    Unavailable,
    /// Shared, so reading it every frame doesn't copy the image.
    Ready(Arc<Preview>),
}

/// An avatar image as the UI sees it.
#[derive(Debug, Clone)]
pub enum AvatarState {
    Loading,
    Unavailable,
    /// A `data:` URL.
    Ready(Arc<str>),
}

/// Composer input that carries a password and must not be kept for recall.
pub fn secret_input(line: &str) -> bool {
    let l = line.trim_start().to_lowercase();
    ["/register", "/verify"]
        .iter()
        .any(|c| l.strip_prefix(c).is_some_and(|rest| rest.is_empty() || !rest.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')))
}

pub struct App {
    pub(crate) env: Env,
    pub zone: Zone,
    pub servers: Vec<ServerConfig>,
    pub settings: Settings,
    pub(crate) conns: BTreeMap<String, Conn>,
    /// Display facts about every connection, so ordering works while one is borrowed out.
    pub(crate) meta: BTreeMap<String, ConnMeta>,
    pub(crate) active: Option<BufId>,
    pub focused: bool,
    pub ready: bool,
    /// Whether the message list is scrolled to the newest line (set by the UI).
    pub at_bottom: bool,
    pub switcher: Option<Switcher>,
    pub search_panel: SearchPanel,
    pub browser: BrowserState,
    /// The connection whose channel browser is open.
    pub browser_conn: Option<String>,
    pub settings_open: Option<SettingsView>,
    pub shortcuts_open: bool,
    /// Latched by the first-run flow so it can show connection progress after the first save.
    pub onboarding: bool,
    pub welcome: WelcomeFlow,
    pub toasts: Vec<Toast>,
    recall: Vec<String>,
    /// conn id → folded channel to open when our JOIN arrives.
    pub(crate) pending_active: BTreeMap<String, String>,
    pub(crate) restore_key: Option<String>,
    pub(crate) blurred_at: i64,
    pub(crate) pending: HashMap<ReqId, Pending>,
    completed: HashMap<ReqId, Result<Reply, String>>,
    pub(crate) searches: HashMap<u64, crate::search::Job>,
    pub(crate) search_pages: HashMap<u64, crate::search::SearchPage>,
    pub(crate) timeline_cache: std::cell::RefCell<HashMap<BufId, (crate::timeline::TimelineKey, Arc<crate::timeline::Timeline>)>>,
    /// The line to flash after a jump (its id).
    pub(crate) flash: Option<u64>,
    previews: HashSet<String>,
    preview_cache: ByteLru<Option<Arc<Preview>>>,
    avatars: HashSet<String>,
    avatar_cache: ByteLru<Option<Arc<str>>>,
    last_prune: i64,
    last_away_check: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ConnMeta {
    pub parent_id: Option<String>,
    pub name: String,
}

impl Default for App {
    fn default() -> Self {
        App::new()
    }
}

impl App {
    pub fn new() -> App {
        App::with_env(Env::new())
    }

    pub fn with_env(env: Env) -> App {
        let now = env.now();
        App {
            env,
            zone: Zone::Local,
            servers: Vec::new(),
            settings: Settings::default(),
            conns: BTreeMap::new(),
            meta: BTreeMap::new(),
            active: None,
            focused: true,
            ready: false,
            at_bottom: true,
            switcher: None,
            search_panel: SearchPanel::default(),
            browser: BrowserState::default(),
            browser_conn: None,
            settings_open: None,
            shortcuts_open: false,
            onboarding: false,
            welcome: WelcomeFlow::default(),
            toasts: Vec::new(),
            recall: Vec::new(),
            pending_active: BTreeMap::new(),
            restore_key: None,
            blurred_at: now,
            pending: HashMap::new(),
            completed: HashMap::new(),
            searches: HashMap::new(),
            search_pages: HashMap::new(),
            timeline_cache: std::cell::RefCell::new(HashMap::new()),
            flash: None,
            previews: HashSet::new(),
            preview_cache: ByteLru::new(500, 48 * 1024 * 1024),
            avatars: HashSet::new(),
            avatar_cache: ByteLru::new(1000, 16 * 1024 * 1024),
            last_prune: now,
            last_away_check: now,
        }
    }

    // ── Clock, effects, change tracking ──────────────────────

    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Epoch milliseconds: the system clock unless [`App::set_now`] pinned it.
    pub fn now(&self) -> i64 {
        self.env.now()
    }

    /// Pin the clock to `ms` (tests and scripted demos); it then stays put until set again.
    pub fn set_now(&self, ms: i64) {
        self.env.set_now(ms);
    }

    /// Everything the model wants done since the last call, in order.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        self.env.take_effects()
    }

    /// Bumps on every change; redraw when it differs from the last one you saw.
    pub fn revision(&self) -> u64 {
        self.env.revision()
    }

    /// When [`App::tick`] next has work, in epoch milliseconds, if ever.
    /// Also wakes once a second while a reconnect countdown is showing.
    pub fn next_wakeup(&self) -> Option<i64> {
        let now = self.now();
        let mut next = self.env.next_timer();
        let mut consider = |t: i64| next = Some(next.map_or(t, |n| n.min(t)));
        let typing = self.conns.values().any(|c| c.all_buffers().any(|b| !b.typing.is_empty()));
        if typing {
            consider(self.last_prune + 2000);
        }
        if self.settings.auto_away_minutes > 0 && !self.focused {
            consider(self.last_away_check + 30_000);
        }
        if self.conns.values().any(|c| c.retry_at > now) {
            consider(now + 1000);
        }
        if !self.toasts.is_empty() {
            // Toasts have their own timers.
        }
        next
    }

    /// Run due timers and periodic housekeeping. Call it when
    /// [`App::next_wakeup`] arrives, or just every few hundred milliseconds.
    pub fn tick(&mut self) {
        for t in self.env.take_due() {
            self.run_timer(t);
        }
        let now = self.now();
        if now - self.last_prune >= 2000 {
            self.last_prune = now;
            self.prune_typing();
        }
        if now - self.last_away_check >= 30_000 {
            self.last_away_check = now;
            self.check_auto_away();
        }
    }

    fn run_timer(&mut self, t: Timer) {
        match t {
            Timer::SaveBuffers { conn } => {
                if let Some(c) = self.conns.get(&conn) {
                    self.env.emit(Effect::SaveBuffers { conn, buffers: crate::saved::saved_list(c) });
                }
            }
            Timer::MarkRead { conn, key } => {
                if let Some(b) = self.conns.get(&conn).and_then(|c| c.buf(&key)) {
                    let line = format!("MARKREAD {} timestamp={}", b.name, crate::irc::iso_time(b.read_marker));
                    self.conns[&conn].send(line);
                }
            }
            Timer::Rejoin { conn } => {
                if let Some(c) = self.conns.get(&conn) {
                    for line in crate::saved::rejoin_lines(c, self.servers.iter().find(|s| s.id == c.server_id)) {
                        c.send(line);
                    }
                }
            }
            Timer::FetchTargets { conn } => {
                self.with_conn(&conn, crate::history::fetch_targets);
            }
            Timer::MetadataSync { conn, target } => {
                if let Some(c) = self.conns.get(&conn) {
                    if c.status == ConnStatus::Connected {
                        c.send(format!("METADATA {target} SYNC"));
                    }
                }
            }
            Timer::ToastExpire { id } => self.dismiss_toast(id),
            Timer::SaveSettings => self.env.emit(Effect::SaveSettings { settings: self.settings.clone() }),
            Timer::SearchDebounce { run } => crate::search_panel::debounce_fired(self, run),
            Timer::ClearFlash { line } => {
                if self.flash == Some(line) {
                    self.flash = None;
                    self.env.touch();
                }
            }
        }
    }

    fn prune_typing(&mut self) {
        let now = self.now();
        let mut changed = false;
        for c in self.conns.values_mut() {
            for b in c.buffers_iter_mut() {
                let before = b.typing.len();
                b.typing.retain(|_, t| *t >= now);
                if b.typing.len() != before {
                    b.touch();
                    changed = true;
                }
            }
        }
        if changed {
            self.env.touch();
        }
    }

    fn check_auto_away(&mut self) {
        let mins = self.settings.auto_away_minutes as i64;
        if mins == 0 || self.focused || self.now() - self.blurred_at < mins * 60_000 {
            return;
        }
        for c in self.conns.values_mut() {
            if c.status != ConnStatus::Connected || c.is_control() || c.away || c.auto_away {
                continue;
            }
            c.auto_away = true;
            c.send("AWAY :Away from keyboard");
        }
    }

    // ── Startup ───────────────────────────────────────────────

    /// Load the saved config and start connecting. `restore_active` is the
    /// last [`App::active_key`] the host persisted (see `Effect::RememberActive`).
    pub fn init(&mut self, config: Config, restore_active: Option<String>) {
        self.servers = config.servers;
        self.settings = Settings::from_value(&config.settings);
        self.restore_key = restore_active.filter(|k| !k.is_empty());
        self.env.emit(Effect::Reset);
        for s in &self.servers {
            if s.autoconnect {
                self.env.emit(Effect::Connect { server: s.id.clone() });
            }
        }
        self.ready = true;
        self.update_welcome();
        self.env.touch();
    }

    /// True while the first-run flow should cover the app.
    pub fn welcoming(&self) -> bool {
        self.ready && (self.servers.is_empty() || self.onboarding)
    }

    // ── Toasts ───────────────────────────────────────────────

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.push_toast(text.into(), kind, None);
    }

    pub fn toast_error(&mut self, text: impl Into<String>) {
        self.push_toast(text.into(), ToastKind::Error, None);
    }

    pub(crate) fn push_toast(&mut self, text: String, kind: ToastKind, action: Option<ToastAction>) {
        let id = self.env.next_seq();
        // Leave time to act on an offer.
        let ttl = if action.is_some() { 20_000 } else { 5000 };
        self.toasts.push(Toast { id, text, kind, action, expires_at: self.now() + ttl });
        self.env.schedule(ttl, Timer::ToastExpire { id });
        self.env.touch();
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
        self.env.touch();
    }

    /// Run the button on a toast (and dismiss it).
    pub fn run_toast_action(&mut self, id: u64) {
        let Some(t) = self.toasts.iter().find(|t| t.id == id).cloned() else { return };
        self.dismiss_toast(id);
        match t.action.map(|a| a.kind) {
            Some(ToastActionKind::SaveLogin { conn, account, password }) => self.save_login(&conn, &account, &password.0),
            None => {}
        }
    }

    pub fn toast_texts(&self) -> Vec<&str> {
        self.toasts.iter().map(|t| t.text.as_str()).collect()
    }

    // ── Lookup ───────────────────────────────────────────────

    pub fn conn(&self, id: &str) -> Option<&Conn> {
        self.conns.get(id)
    }

    pub fn conn_mut(&mut self, id: &str) -> Option<&mut Conn> {
        self.conns.get_mut(id)
    }

    pub fn conns(&self) -> impl Iterator<Item = &Conn> {
        self.conns.values()
    }

    pub fn buffer(&self, id: &BufId) -> Option<&Buffer> {
        self.conns.get(&id.conn)?.buf(&id.key)
    }

    pub fn buffer_mut(&mut self, id: &BufId) -> Option<&mut Buffer> {
        self.conns.get_mut(&id.conn)?.buf_mut(&id.key)
    }

    /// The open buffer's id.
    pub fn active(&self) -> Option<&BufId> {
        self.active.as_ref()
    }

    pub fn active_buffer(&self) -> Option<&Buffer> {
        self.buffer(self.active.as_ref()?)
    }

    pub fn active_conn(&self) -> Option<&Conn> {
        self.conns.get(&self.active.as_ref()?.conn)
    }

    /// Run `f` with the connection taken out of the app, so both can be
    /// borrowed mutably. Nothing inside may look the same connection up by id.
    pub(crate) fn with_conn<R>(&mut self, id: &str, f: impl FnOnce(&mut App, &mut Conn) -> R) -> Option<R> {
        let mut conn = self.conns.remove(id)?;
        let r = f(self, &mut conn);
        self.conns.insert(id.to_string(), conn);
        Some(r)
    }

    /// A buffer by id while `conn` is borrowed out.
    pub(crate) fn buffer_in<'a>(&'a mut self, conn: &'a mut Conn, id: &BufId) -> Option<&'a mut Buffer> {
        if id.conn == conn.id {
            conn.buf_mut(&id.key)
        } else {
            self.conns.get_mut(&id.conn)?.buf_mut(&id.key)
        }
    }

    /// Root connections in config order, each followed by its bouncer children.
    pub fn ordered_conn_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.servers {
            if !self.meta.contains_key(&s.id) {
                continue;
            }
            out.push(s.id.clone());
            let mut kids: Vec<(&String, &ConnMeta)> = self.meta.iter().filter(|(_, m)| m.parent_id.as_deref() == Some(s.id.as_str())).collect();
            kids.sort_by(|a, b| crate::buffer::collate(&a.1.name, &b.1.name));
            out.extend(kids.into_iter().map(|(id, _)| id.clone()));
        }
        out
    }

    pub fn ordered_conns(&self) -> Vec<&Conn> {
        self.ordered_conn_ids().iter().filter_map(|id| self.conns.get(id)).collect()
    }

    /// Every buffer as the sidebar lists them: per connection, the server log then channels and queries.
    pub fn all_buffer_ids(&self) -> Vec<BufId> {
        let mut out = Vec::new();
        for id in self.ordered_conn_ids() {
            if let Some(c) = self.conns.get(&id) {
                out.push(BufId::server(&c.id));
                out.extend(c.channel_buffers().iter().map(|b| b.id()));
            }
        }
        out
    }

    /// Like [`App::all_buffer_ids`] while `conn` is borrowed out.
    pub(crate) fn all_buffer_ids_in(&self, conn: &Conn) -> Vec<BufId> {
        let mut out = Vec::new();
        for id in self.ordered_conn_ids() {
            let c = if id == conn.id { Some(conn) } else { self.conns.get(&id) };
            if let Some(c) = c {
                out.push(BufId::server(&c.id));
                out.extend(c.channel_buffers().iter().map(|b| b.id()));
            }
        }
        out
    }

    pub(crate) fn refresh_names(&mut self) {
        let ids: Vec<String> = self.conns.keys().cloned().collect();
        for id in ids {
            let name = self.compute_name(&self.conns[&id]);
            let c = self.conns.get_mut(&id).unwrap();
            c.set_name(name.clone());
            self.meta.insert(id, ConnMeta { parent_id: c.parent_id.clone(), name });
        }
    }

    pub(crate) fn refresh_name_in(&mut self, conn: &mut Conn) {
        let name = self.compute_name(conn);
        conn.set_name(name.clone());
        self.meta.insert(conn.id.clone(), ConnMeta { parent_id: conn.parent_id.clone(), name });
    }

    fn compute_name(&self, c: &Conn) -> String {
        let cfg = self.servers.iter().find(|s| s.id == c.server_id);
        if let (Some(netid), Some(parent)) = (&c.netid, &c.parent_id) {
            if let Some(name) = self.conns.get(parent).and_then(|p| p.networks.get(netid)).and_then(|a| a.get("name")).filter(|n| !n.is_empty()) {
                return name.clone();
            }
            if let Some(n) = c.isupport.get("NETWORK").filter(|n| !n.is_empty()) {
                return n.clone();
            }
        }
        [cfg.map(|s| s.name.as_str()), c.isupport.get("NETWORK").map(String::as_str), cfg.map(|s| s.host.as_str())]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| c.id.clone())
    }

    // ── Navigation ───────────────────────────────────────────

    /// Open a buffer.
    pub fn open(&mut self, id: &BufId) {
        let key = id.key.clone();
        self.with_conn(&id.conn, |app, conn| app.open_in(conn, &key));
    }

    pub(crate) fn open_in(&mut self, conn: &mut Conn, key: &str) {
        let id = conn.buf_id(key);
        if self.active.as_ref() == Some(&id) || conn.buf(key).is_none() {
            return;
        }
        if let Some(prev) = self.active.clone() {
            if let Some(b) = self.buffer_in(conn, &prev) {
                b.divider = 0;
                b.touch();
            }
        }
        self.active = Some(id);
        let b = conn.buf_mut(key).unwrap();
        b.divider = if b.unread() > 0 { b.read_marker } else { 0 };
        b.touch();
        let (kind, idle, n) = (b.kind, b.history == HistoryState::Idle, b.lines().len());
        self.remember_active_in(conn);
        if kind != BufferKind::Server && idle && n < 30 {
            crate::history::fetch_history(self, conn, key, HistMode::Before);
        }
        self.env.touch();
    }

    /// Persist which buffer is open, to restore it on the next launch.
    pub(crate) fn remember_active_in(&self, conn: &Conn) {
        if let Some(a) = self.active.as_ref().filter(|a| a.conn == conn.id) {
            self.env.emit(Effect::RememberActive { key: format!("{}|{}|{}", conn.server_id, conn.netid.as_deref().unwrap_or(""), a.key) });
        }
    }

    pub fn active_key(&self) -> Option<String> {
        let a = self.active.as_ref()?;
        let c = self.conns.get(&a.conn)?;
        Some(format!("{}|{}|{}", c.server_id, c.netid.as_deref().unwrap_or(""), a.key))
    }

    /// Open the neighbouring buffer, optionally skipping those with nothing unread.
    pub fn step(&mut self, delta: i32, unread_only: bool) {
        let list: Vec<BufId> =
            self.all_buffer_ids().into_iter().filter(|id| id.is_server() || !self.conns.get(&id.conn).is_some_and(|c| c.collapsed)).collect();
        if list.is_empty() {
            return;
        }
        let n = list.len() as i32;
        let mut i = self.active.as_ref().and_then(|a| list.iter().position(|x| x == a)).map_or(-1, |p| p as i32);
        for _ in 0..n {
            i = (i + delta + n).rem_euclid(n);
            let id = &list[i as usize];
            if !unread_only || self.summary(id).unread > 0 {
                let id = id.clone();
                return self.open(&id);
            }
        }
    }

    pub fn mark_read(&mut self, id: &BufId) {
        let key = id.key.clone();
        if let Some(c) = self.conns.get_mut(&id.conn) {
            crate::unread::mark_read(c, &key);
        }
    }

    /// Load older (or newer) history for a buffer.
    pub fn fetch_history(&mut self, id: &BufId, mode: HistMode) {
        let key = id.key.clone();
        self.with_conn(&id.conn, |app, conn| crate::history::fetch_history(app, conn, &key, mode));
    }

    /// Whether a gap in the open buffer can be filled now (history is available).
    pub fn can_fill_gap(&self, id: &BufId) -> bool {
        self.conns.get(&id.conn).is_some_and(|c| crate::history::can_fill(self, c))
    }

    /// Load a page of what's missing in `gap`: `GapFrom::After` from its older edge (scrolling down into it), `Before` from its newer one.
    pub fn fill_gap(&mut self, id: &BufId, gap: crate::buffer::Gap, from: GapFrom) {
        let key = id.key.clone();
        self.with_conn(&id.conn, |app, conn| crate::history::fetch_gap(app, conn, &key, gap, from));
    }

    /// A buffer was renamed (`old` to `new`): follow it in everything that names buffers by key.
    pub(crate) fn rekey(&mut self, conn_id: &str, old: &str, new: &str) {
        if old == new {
            return;
        }
        let same = |id: &BufId| id.conn == conn_id && id.key == old;
        let to = |id: &mut BufId| id.key = new.to_string();
        if let Some(a) = self.active.as_mut().filter(|a| same(a)) {
            to(a);
        }
        if let Some(o) = self.search_panel.origin.as_mut().filter(|o| same(o)) {
            to(o);
        }
        if let Some(k) = self.pending_active.get_mut(conn_id).filter(|k| *k == old) {
            *k = new.to_string();
        }
        self.timeline_cache.borrow_mut().remove(&BufId::new(conn_id, old));
        for p in self.pending.values_mut() {
            match p {
                Pending::History { buf, .. } | Pending::Around { buf, .. } | Pending::Gap { buf, .. } | Pending::Upload { buf } if same(buf) => to(buf),
                _ => {}
            }
        }
        self.env.touch();
    }

    pub fn close_buffer(&mut self, id: &BufId) {
        let key = id.key.clone();
        self.with_conn(&id.conn, |app, conn| app.close_buffer_in(conn, &key));
    }

    pub(crate) fn close_buffer_in(&mut self, conn: &mut Conn, key: &str) {
        let Some(b) = conn.buf(key) else { return };
        if b.kind == BufferKind::Server {
            return;
        }
        let (kind, joined, name) = (b.kind, b.joined, b.name.clone());
        if kind == BufferKind::Channel && joined {
            conn.send(format!("PART {name}"));
        }
        if let Some(b) = conn.buf_mut(key) {
            b.want_joined = false;
        }
        let list = self.all_buffer_ids_in(conn);
        let id = conn.buf_id(key);
        let i = list.iter().position(|x| *x == id);
        conn.remove_buffer(key);
        if kind == BufferKind::Query && conn.can_monitor() && !crate::monitor::is_watched(self, conn, &name) {
            conn.send(format!("MONITOR - {name}"));
        }
        conn.save_buffers_soon();
        if self.active.as_ref() == Some(&id) {
            self.active = None;
            let next = i.and_then(|i| i.checked_sub(1).and_then(|p| list.get(p)).or_else(|| list.get(i + 1)));
            match next {
                Some(n) if n.conn == conn.id => {
                    let k = n.key.clone();
                    self.open_in(conn, &k);
                }
                Some(n) => {
                    let n = n.clone();
                    self.open_other(&n);
                }
                None => {}
            }
        }
        self.env.touch();
    }

    /// Open a buffer on a connection other than the one borrowed out.
    fn open_other(&mut self, id: &BufId) {
        self.open(id);
    }

    /// The saved-buffer restore found the buffer the user had open last time.
    pub(crate) fn maybe_restore_active(&mut self, conn: &mut Conn, key: &str) {
        let Some(want) = self.restore_key.clone() else { return };
        if want == format!("{}|{}|{}", conn.server_id, conn.netid.as_deref().unwrap_or(""), key) {
            self.restore_key = None;
            let target = if key.is_empty() || conn.buf(key).is_none() { String::new() } else { key.to_string() };
            self.open_in(conn, &target);
        }
    }

    // ── Settings & servers ───────────────────────────────────

    /// Change settings; they are saved shortly after the last change.
    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        self.env.schedule(300, Timer::SaveSettings);
        self.env.touch();
    }

    /// Save a server and (optionally) connect to it. Returns the request id;
    /// the outcome is available from [`App::take_result`] once resolved.
    pub fn save_server(&mut self, server: ServerConfig, connect_now: bool) -> ReqId {
        self.save_server_for(server, connect_now, None)
    }

    fn save_server_for(&mut self, server: ServerConfig, connect_now: bool, login: Option<(String, String)>) -> ReqId {
        let req = self.env.next_req();
        self.pending.insert(req, Pending::SaveServer { connect_now, login });
        self.env.emit(Effect::SaveServer { req, server: Secret(server) });
        req
    }

    pub fn delete_server(&mut self, id: &str) {
        self.env.emit(Effect::DeleteServer { id: id.to_string() });
        self.servers.retain(|s| s.id != id);
        let doomed: Vec<String> = self.conns.values().filter(|c| c.server_id == id).map(|c| c.id.clone()).collect();
        for c in doomed {
            self.remove_conn(&c);
        }
        self.refresh_names();
        self.update_welcome();
    }

    pub(crate) fn remove_conn(&mut self, id: &str) {
        if self.conns.remove(id).is_none() {
            return;
        }
        self.meta.remove(id);
        if self.active.as_ref().is_some_and(|a| a.conn == id) {
            self.active = None;
            if let Some(first) = self.all_buffer_ids().into_iter().next() {
                self.open(&first);
            }
        }
        self.env.touch();
    }

    /// The effective notification level for a buffer: `None` means "use the default".
    pub fn notify_level(&self, b: &Buffer) -> Option<NotifyLevel> {
        self.settings.buffer_notify.get(&b.pref_key()).copied()
    }

    pub fn set_notify(&mut self, id: &BufId, level: Option<NotifyLevel>) {
        let Some(key) = self.buffer(id).map(|b| b.pref_key()) else { return };
        self.set_notify_pref(key, level);
    }

    /// [`App::set_notify`] while `conn` is borrowed out of the app.
    pub(crate) fn set_notify_in(&mut self, conn: &Conn, key: &str, level: Option<NotifyLevel>) {
        let Some(pref) = conn.buf(key).map(|b| b.pref_key()) else { return };
        self.set_notify_pref(pref, level);
    }

    fn set_notify_pref(&mut self, key: String, level: Option<NotifyLevel>) {
        self.update_settings(|s| match level {
            None => {
                s.buffer_notify.remove(&key);
            }
            Some(l) => {
                s.buffer_notify.insert(key, l);
            }
        });
    }

    /// Unread count, mention count and level for the sidebar and switcher.
    pub fn summary(&self, id: &BufId) -> BufSummary {
        match self.buffer(id) {
            Some(b) => {
                let level = self.notify_level(b);
                BufSummary { unread: b.unread(), highlights: b.highlights(level), level }
            }
            None => BufSummary::default(),
        }
    }

    /// Mentions and private messages waiting across every buffer, for the dock badge.
    pub fn attention(&self) -> u32 {
        self.conns.values().flat_map(|c| c.all_buffers()).map(|b| b.highlights(self.notify_level(b)) as u32).sum()
    }

    // ── Ignore & highlights ──────────────────────────────────

    pub fn is_ignored(&self, conn: &Conn, src: Option<&Source>) -> bool {
        let Some(src) = src else { return false };
        if conn.is_me(&src.nick) {
            return false;
        }
        is_ignored(&self.settings.ignores, src, |s| conn.fold(s))
    }

    pub fn ignore(&mut self, mask: &str) {
        let m = normalize_mask(mask);
        if m.is_empty() || self.settings.ignores.contains(&m) {
            return;
        }
        self.update_settings(|s| s.ignores.push(m));
    }

    /// Returns whether anything was removed.
    pub fn unignore(&mut self, mask: &str) -> bool {
        let m = normalize_mask(mask);
        let before = self.settings.ignores.len();
        let next: Vec<String> = self.settings.ignores.iter().filter(|x| **x != m && *x != mask).cloned().collect();
        if next.len() == before {
            return false;
        }
        self.update_settings(|s| s.ignores = next);
        true
    }

    pub fn network_highlights(&self, server_id: &str) -> NetworkHighlights {
        self.settings.network_highlights.get(server_id).cloned().unwrap_or_default()
    }

    /// Words that highlight on this connection: our nick, global words, and the network's.
    pub fn highlight_words(&self, conn: &Conn) -> Vec<String> {
        let mut v = vec![conn.nick.clone()];
        v.extend(self.settings.highlight_words.iter().cloned());
        if let Some(n) = self.settings.network_highlights.get(&conn.server_id) {
            v.extend(n.words.iter().cloned());
        }
        v
    }

    /// Whether a message from `src` highlights us.
    pub fn is_highlight(&self, conn: &Conn, src: Option<&Source>, text: &str) -> bool {
        if let (Some(src), Some(n)) = (src, self.settings.network_highlights.get(&conn.server_id)) {
            if is_ignored(&n.never, src, |s| conn.fold(s)) {
                return false;
            }
        }
        mentions(text, &self.highlight_words(conn))
    }

    /// Decides whether a message deserves a system notification and queues it.
    pub(crate) fn maybe_notify_in(&mut self, conn: &Conn, key: &str, nick: &str, text: &str, important: bool) {
        let Some(b) = conn.buf(key) else { return };
        let level = self.notify_level(b);
        let is_query = b.kind == BufferKind::Query;
        let is_active = self.active.as_ref() == Some(&conn.buf_id(key));
        if !crate::notify::should_notify(&self.settings, level, is_query, important, self.focused && is_active) {
            return;
        }
        let place = if is_query { nick.to_string() } else { format!("{nick} in {}", b.name) };
        self.env.emit(crate::notify::notification(&format!("{place} · {}", conn.name()), &strip_formatting(text)));
    }

    // ── Presence ─────────────────────────────────────────────

    /// The window gained or lost focus.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.env.touch();
        if !focused {
            self.blurred_at = self.now();
            return;
        }
        for c in self.conns.values_mut() {
            if c.auto_away {
                c.auto_away = false;
                if c.away && c.status == ConnStatus::Connected {
                    c.send("AWAY");
                }
            }
        }
    }

    // ── Outgoing ─────────────────────────────────────────────

    /// Send `text` to a buffer as one or more PRIVMSG/NOTICE lines.
    pub fn send_message(&mut self, id: &BufId, text: &str, kind: SendKind, action: bool, reply_to: Option<&str>) {
        let key = id.key.clone();
        let reply = reply_to.map(String::from);
        self.with_conn(&id.conn, |app, conn| app.send_message_in(conn, &key, text, kind, action, reply.as_deref()));
    }

    pub(crate) fn send_message_in(&mut self, conn: &mut Conn, key: &str, text: &str, kind: SendKind, action: bool, reply_to: Option<&str>) {
        let Some(buf) = conn.buf(key) else { return };
        let target = buf.name.clone();
        let reply = reply_to.filter(|_| conn.can_tag("draft/reply"));
        let cmd = kind.command();
        // Leave room for ":nick!user@host CMD target :" plus CTCP wrapping.
        let budget = 510_i64 - (conn.nick.len() + 100 + cmd.len() + target.len() + if action { 9 } else { 0 }) as i64;
        let echo = conn.has("echo-message");
        for (i, part) in split_message(text, budget.max(60) as usize).into_iter().enumerate() {
            let body = if action { format!("\x01ACTION {part}\x01") } else { part.clone() };
            let tags = match reply.filter(|_| i == 0) {
                Some(r) => format!("@+draft/reply={} ", escape_tag(r)),
                None => String::new(),
            };
            conn.send(format!("{tags}{cmd} {target} :{body}"));
            if !echo {
                let mut line = self.env.line(if action {
                    LineType::Action
                } else if kind == SendKind::Notice {
                    LineType::Notice
                } else {
                    LineType::Privmsg
                });
                line.nick = conn.nick.clone();
                line.text = part;
                line.is_self = true;
                line.reply_to = reply.filter(|_| i == 0).map(String::from);
                if let Some(b) = conn.buf_mut(key) {
                    b.push(line);
                }
            }
        }
        if let Some(b) = conn.buf_mut(key) {
            b.last_typing_sent = 0;
        }
    }

    /// Send text with line breaks as draft/multiline batches, so it arrives as
    /// one message. Returns false when the server can't take it that way.
    pub fn send_multiline(&mut self, id: &BufId, text: &str, reply_to: Option<&str>) -> bool {
        let key = id.key.clone();
        let reply = reply_to.map(String::from);
        self.with_conn(&id.conn, |app, conn| app.send_multiline_in(conn, &key, text, reply.as_deref())).unwrap_or(false)
    }

    pub(crate) fn send_multiline_in(&mut self, conn: &mut Conn, key: &str, text: &str, reply_to: Option<&str>) -> bool {
        let Some(buf) = conn.buf(key) else { return false };
        let Some(limits) = parse_multiline_cap(conn.cap_values.get("draft/multiline").map(String::as_str)) else { return false };
        if !conn.has("batch") || buf.kind == BufferKind::Server {
            return false;
        }
        // A single line (maybe with stray line breaks around it) needs no batch.
        if text.split('\n').filter(|l| !l.trim().is_empty()).count() < 2 {
            return false;
        }
        let target = buf.name.clone();
        let budget = 510_i64 - (conn.nick.len() + 100 + "PRIVMSG".len() + target.len()) as i64;
        let batches = multiline_batches(text, limits, budget.max(60) as usize);
        if batches.is_empty() {
            return false;
        }
        let reply = reply_to.filter(|_| conn.can_tag("draft/reply"));
        let echo = conn.has("echo-message");
        for (i, parts) in batches.iter().enumerate() {
            let r = format!("ml{}", self.env.next_seq());
            // Labeled, so a FAIL about the batch (too long, say) lands here.
            let label = conn.label_for(key);
            let mut tags: Vec<String> = Vec::new();
            if let Some(l) = label {
                tags.push(format!("label={l}"));
            }
            if let (Some(rep), 0) = (reply, i) {
                tags.push(format!("+draft/reply={}", escape_tag(rep)));
            }
            let head = if tags.is_empty() { String::new() } else { format!("@{} ", tags.join(";")) };
            let mut lines = vec![format!("{head}BATCH +{r} draft/multiline {target}")];
            for p in parts {
                let concat = if p.concat { ";draft/multiline-concat" } else { "" };
                lines.push(format!("@batch={r}{concat} PRIVMSG {target} :{}", p.text));
            }
            lines.push(format!("BATCH -{r}"));
            conn.send_lines(lines);
            if !echo {
                let mut line = self.env.line(LineType::Privmsg);
                line.nick = conn.nick.clone();
                line.text = join_parts(parts);
                line.is_self = true;
                line.reply_to = reply.filter(|_| i == 0).map(String::from);
                if let Some(b) = conn.buf_mut(key) {
                    b.push(line);
                }
            }
        }
        if let Some(b) = conn.buf_mut(key) {
            b.last_typing_sent = 0;
        }
        true
    }

    /// Tell the channel we're typing (or stopped), at most every 3 seconds.
    pub fn send_typing(&mut self, id: &BufId, state: TypingState) {
        if !self.settings.send_typing || id.is_server() {
            return;
        }
        let now = self.now();
        let Some(conn) = self.conns.get_mut(&id.conn) else { return };
        if !conn.can_tag("typing") {
            return;
        }
        let Some(buf) = conn.buf_mut(&id.key) else { return };
        match state {
            TypingState::Active if now - buf.last_typing_sent < 3000 => return,
            TypingState::Done if buf.last_typing_sent == 0 => return,
            _ => {}
        }
        buf.last_typing_sent = if state == TypingState::Active { now } else { 0 };
        let name = buf.name.clone();
        conn.send(format!("@+typing={} TAGMSG {name}", if state == TypingState::Active { "active" } else { "done" }));
    }

    pub fn can_react(&self, id: &BufId, line: &Line) -> bool {
        let Some(c) = self.conns.get(&id.conn) else { return false };
        line.msgid.is_some() && !id.is_server() && c.can_tag("draft/react") && c.can_tag("draft/reply")
    }

    pub fn react(&mut self, id: &BufId, line: &Line, emoji: &str) {
        let Some(msgid) = line.msgid.clone() else { return };
        let Some(conn) = self.conns.get_mut(&id.conn) else { return };
        if line.reactions.iter().find(|(e, _)| e == emoji).is_some_and(|(_, who)| who.iter().any(|n| conn.is_me(n))) {
            return;
        }
        let Some(name) = conn.buf(&id.key).map(|b| b.name.clone()) else { return };
        conn.send(format!("@+draft/react={};+draft/reply={} TAGMSG {name}", escape_tag(emoji), escape_tag(&msgid)));
        if !conn.has("echo-message") {
            let me = conn.nick.clone();
            if let Some(b) = conn.buf_mut(&id.key) {
                b.react(&msgid, emoji, &me);
            }
        }
    }

    pub fn can_redact(&self, id: &BufId, line: &Line) -> bool {
        let Some(conn) = self.conns.get(&id.conn) else { return false };
        if line.msgid.is_none() || !conn.has("draft/message-redaction") {
            return false;
        }
        if line.is_self {
            return true;
        }
        let me = conn.buf(&id.key).and_then(|b| b.member(&conn.nick)).and_then(|m| m.top_prefix());
        let op = conn.prefix().rank('@');
        match (me.and_then(|c| conn.prefix().rank(c)), op) {
            (Some(mine), Some(op)) => mine <= op,
            _ => false,
        }
    }

    pub fn redact(&mut self, id: &BufId, line: &Line) {
        if let (Some(msgid), Some(conn)) = (&line.msgid, self.conns.get(&id.conn)) {
            if let Some(b) = conn.buf(&id.key) {
                conn.send(format!("REDACT {} {msgid}", b.name));
            }
        }
    }

    pub fn join(&mut self, conn_id: &str, channel: &str, key: Option<&str>) {
        let (channel, key) = (channel.to_string(), key.map(String::from));
        self.with_conn(conn_id, |app, conn| app.join_in(conn, &channel, key.as_deref()));
    }

    pub(crate) fn join_in(&mut self, conn: &mut Conn, channel: &str, key: Option<&str>) {
        self.pending_active.insert(conn.id.clone(), conn.fold(channel));
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            conn.pending_keys.insert(conn.fold(channel), k.to_string());
        }
        if let Some(b) = conn.buffer(channel).filter(|b| b.joined) {
            let k = b.key();
            return self.open_in(conn, &k);
        }
        match key.filter(|k| !k.is_empty()) {
            Some(k) => conn.send(format!("JOIN {channel} {k}")),
            None => conn.send(format!("JOIN {channel}")),
        }
    }

    /// Open (and focus) a conversation with `nick`.
    pub fn query(&mut self, conn_id: &str, nick: &str) -> Option<BufId> {
        let nick = nick.to_string();
        self.with_conn(conn_id, |app, conn| app.query_in(conn, &nick))
    }

    pub(crate) fn query_in(&mut self, conn: &mut Conn, nick: &str) -> BufId {
        let key = conn.ensure(nick, Some(BufferKind::Query));
        self.open_in(conn, &key);
        conn.buf_id(&key)
    }

    /// Open the channel browser, asking the server for its list if ours is stale.
    pub fn browse_channels(&mut self, conn_id: &str) {
        self.with_conn(conn_id, |app, conn| app.browse_channels_in(conn));
    }

    pub(crate) fn browse_channels_in(&mut self, conn: &mut Conn) {
        self.browser_conn = Some(conn.id.clone());
        let now = self.now();
        let stale = match &conn.channel_list {
            None => true,
            Some(l) => !l.loading && now - l.at > 10 * 60_000,
        };
        if stale && conn.status == ConnStatus::Connected {
            let items = conn.channel_list.take().map(|l| l.items).unwrap_or_default();
            conn.channel_list = Some(crate::conn::ChannelList { loading: true, items, at: now });
            conn.send("LIST");
        }
        self.env.touch();
    }

    /// Forget the channel list and fetch it again.
    pub fn refresh_channel_list(&mut self, conn_id: &str) {
        if let Some(c) = self.conns.get_mut(conn_id) {
            c.channel_list = None;
        }
        self.browse_channels(conn_id);
    }

    pub fn close_channel_browser(&mut self) {
        self.browser_conn = None;
        self.env.touch();
    }

    // ── Link previews & avatars ──────────────────────────────

    /// Ask for a link preview (once per URL); read it with [`App::preview`].
    pub fn request_preview(&mut self, url: &str) {
        if self.previews.contains(url) || self.preview_cache.has(url) {
            return;
        }
        let req = self.env.next_req();
        self.previews.insert(url.to_string());
        self.pending.insert(req, Pending::Preview { url: url.to_string() });
        self.env.emit(Effect::LinkPreview { req, url: url.to_string() });
    }

    pub fn preview(&self, url: &str) -> PreviewState {
        match self.preview_cache.get(url) {
            Some(Some(p)) => PreviewState::Ready(p.clone()),
            Some(None) => PreviewState::Unavailable,
            None if self.previews.contains(url) => PreviewState::Loading,
            None => PreviewState::Unavailable,
        }
    }

    /// Ask for a metadata avatar (only if the privacy setting allows); read it with [`App::avatar`].
    pub fn request_avatar(&mut self, url: &str) {
        if !self.settings.load_avatars || self.avatars.contains(url) || self.avatar_cache.has(url) {
            return;
        }
        let req = self.env.next_req();
        self.avatars.insert(url.to_string());
        self.pending.insert(req, Pending::Avatar { url: url.to_string() });
        self.env.emit(Effect::Avatar { req, url: url.to_string() });
    }

    pub fn avatar(&self, url: &str) -> AvatarState {
        match self.avatar_cache.get(url) {
            Some(Some(d)) => AvatarState::Ready(d.clone()),
            Some(None) => AvatarState::Unavailable,
            None if self.avatars.contains(url) => AvatarState::Loading,
            None => AvatarState::Unavailable,
        }
    }

    /// Use the account from /register for SASL on this server from now on.
    pub fn save_login(&mut self, conn_id: &str, account: &str, password: &str) {
        let Some(server_id) = self.conns.get(conn_id).map(|c| c.server_id.clone()) else { return };
        let Some(mut cfg) = self.servers.iter().find(|s| s.id == server_id).cloned() else { return };
        cfg.sasl = crate::types::SaslMechanism::Plain;
        cfg.sasl_username = account.to_string();
        cfg.sasl_password = password.to_string();
        let name = self.conns.get(conn_id).map(|c| c.name().to_string()).unwrap_or_default();
        // Reported when the save lands.
        self.save_server_for(cfg, false, Some((account.to_string(), name)));
    }

    // ── Composer helpers ─────────────────────────────────────

    pub fn input_history(&self) -> &[String] {
        &self.recall
    }

    pub fn set_input_history(&mut self, v: Vec<String>) {
        self.recall = v.into_iter().filter(|h| !secret_input(h)).collect();
    }

    pub(crate) fn push_input_history(&mut self, value: &str) {
        self.recall.retain(|h| h != value);
        self.recall.insert(0, value.to_string());
        self.recall.truncate(100);
    }

    // ── Results ──────────────────────────────────────────────

    /// The outcome of a request the UI is waiting on (save server, cert
    /// generation, …). Removes it.
    pub fn take_result(&mut self, req: ReqId) -> Option<Result<Reply, String>> {
        self.completed.remove(&req)
    }

    pub(crate) fn complete(&mut self, req: ReqId, result: Result<Reply, String>) {
        self.completed.insert(req, result);
        if self.completed.len() > 100 {
            if let Some(k) = self.completed.keys().min().copied() {
                self.completed.remove(&k);
            }
        }
    }

    /// Ask the backend about a client certificate file (answer via [`App::take_result`]).
    pub fn request_cert_info(&mut self, path: &str) -> ReqId {
        let req = self.env.next_req();
        self.pending.insert(req, Pending::Ui);
        self.env.emit(Effect::CertInfo { req, path: path.to_string() });
        req
    }

    pub fn request_generate_cert(&mut self, server: &str) -> ReqId {
        let req = self.env.next_req();
        self.pending.insert(req, Pending::Ui);
        self.env.emit(Effect::GenerateCert { req, server: server.to_string() });
        req
    }

    /// Delete the local message log (Settings → Privacy & data).
    pub fn clear_logs(&mut self) {
        let req = self.env.next_req();
        self.pending.insert(req, Pending::ClearLogs);
        self.env.emit(Effect::ClearLogs { req });
    }

    /// Upload a file's bytes to the server's file host; the link lands in the composer.
    pub fn upload_bytes(&mut self, id: &BufId, name: &str, content_type: &str, data: Vec<u8>) {
        let Some(conn) = self.conns.get(&id.conn) else { return };
        let url = self.filehost(&id.conn).unwrap_or_default();
        if url.is_empty() {
            return self.toast_error("This server does not accept file uploads");
        }
        let server = conn.server_id.clone();
        let req = self.env.next_req();
        self.pending.insert(req, Pending::Upload { buf: id.clone() });
        if let Some(b) = self.buffer_mut(id) {
            b.composer.uploading += 1;
            b.touch();
        }
        self.env.emit(Effect::UploadBytes { req, server, url, name: name.to_string(), content_type: content_type.to_string(), data: Secret(data) });
    }

    /// Upload a file by path.
    pub fn upload_file(&mut self, id: &BufId, path: &str) {
        let Some(conn) = self.conns.get(&id.conn) else { return };
        let url = self.filehost(&id.conn).unwrap_or_default();
        if url.is_empty() {
            return self.toast_error("This server does not accept file uploads");
        }
        let server = conn.server_id.clone();
        let req = self.env.next_req();
        self.pending.insert(req, Pending::Upload { buf: id.clone() });
        if let Some(b) = self.buffer_mut(id) {
            b.composer.uploading += 1;
            b.touch();
        }
        self.env.emit(Effect::UploadFile { req, server, url, path: path.to_string() });
    }

    /// Whether files can go up from this buffer: the server advertises a file host and the buffer is a conversation.
    pub fn can_upload(&self, id: &BufId) -> bool {
        self.buffer(id).is_some_and(|b| b.kind != BufferKind::Server) && self.filehost(&id.conn).is_some()
    }

    /// The file-upload endpoint this connection's server advertises, if any.
    pub fn filehost(&self, conn_id: &str) -> Option<String> {
        let c = self.conns.get(conn_id)?;
        c.isupport.get("soju.im/FILEHOST").or_else(|| c.isupport.get("FILEHOST")).cloned().filter(|s| !s.is_empty())
    }

    /// Feed back the answer to a request from [`Effect`].
    pub fn resolve(&mut self, req: ReqId, result: Result<Reply, String>) {
        let Some(p) = self.pending.remove(&req) else { return };
        match p {
            Pending::History { buf, mode, limit } => crate::history::history_done(self, req, buf, mode, limit, result),
            Pending::Around { buf, time, msgid, stage } => crate::history::around_done(self, buf, time, msgid, stage, result),
            Pending::Gap { buf, gap, from, remote, limit } => crate::history::gap_done(self, buf, gap, from, remote, limit, result),
            Pending::Targets { conn } => crate::history::targets_done(self, &conn, result),
            Pending::Restore { conn } => crate::saved::restore_done(self, &conn, result),
            Pending::SaveServer { connect_now, login } => {
                self.save_server_done(connect_now, login, &result);
                self.complete(req, result);
            }
            Pending::Search(step) => crate::search::step_done(self, step, result),
            Pending::Preview { url } => {
                let r = match result {
                    Ok(Reply::Preview(p)) => Some(Arc::new(p)),
                    _ => None,
                };
                let bytes = r.as_ref().and_then(|p| p.image.as_ref()).map_or(0, |i| i.len());
                self.previews.remove(&url);
                self.preview_cache.set(&url, r, bytes);
            }
            Pending::Avatar { url } => {
                let r: Option<Arc<str>> = match result {
                    Ok(Reply::Text(d)) => Some(Arc::from(d)),
                    _ => None,
                };
                let bytes = r.as_ref().map_or(0, |d| d.len());
                self.avatars.remove(&url);
                self.avatar_cache.set(&url, r, bytes);
            }
            Pending::ClearLogs => match result {
                Ok(_) => self.toast("Saved messages deleted", ToastKind::Info),
                Err(e) => self.toast_error(e),
            },
            Pending::Upload { buf } => {
                if let Some(b) = self.buffer_mut(&buf) {
                    b.composer.uploading = b.composer.uploading.saturating_sub(1);
                    b.touch();
                }
                match result {
                    Ok(Reply::Text(url)) => self.composer_insert(&buf, &url),
                    Ok(_) => {}
                    Err(e) => self.toast_error(format!("Upload failed: {e}")),
                }
            }
            Pending::Ui => self.complete(req, result),
        }
        self.update_welcome();
        self.poll_search();
        self.env.touch();
    }

    fn save_server_done(&mut self, connect_now: bool, login: Option<(String, String)>, result: &Result<Reply, String>) {
        if let Some((account, name)) = login {
            match result {
                Ok(_) => self.toast(format!("Hearth will sign in as {account} on {name}"), ToastKind::Info),
                Err(e) => self.toast_error(format!("Couldn't save the login: {e}")),
            }
        }
        let Ok(Reply::Server(saved)) = result else { return };
        match self.servers.iter_mut().find(|s| s.id == saved.id) {
            Some(s) => *s = saved.clone(),
            None => self.servers.push(saved.clone()),
        }
        if connect_now {
            self.env.emit(Effect::Disconnect { server: saved.id.clone(), reason: None });
            self.env.emit(Effect::Connect { server: saved.id.clone() });
        }
        self.refresh_names();
    }

    /// A `Send` effect failed: show why in the buffer that was active.
    pub fn send_failed(&mut self, conn: &str, error: &str) {
        let id = self.active.clone().filter(|a| a.conn == conn).unwrap_or_else(|| BufId::server(conn));
        let mut line = self.env.line(LineType::Error);
        line.text = error.to_string();
        if let Some(b) = self.buffer_mut(&id) {
            b.push(line);
        }
    }

    /// Show a line in a buffer as an error or info message.
    pub fn say(&mut self, id: &BufId, kind: LineType, text: &str) {
        let mut line = self.env.line(kind);
        line.text = text.to_string();
        if let Some(b) = self.buffer_mut(id) {
            b.push(line);
        }
    }

    // ── Incoming ─────────────────────────────────────────────

    pub fn handle_events(&mut self, events: &[IrcEvent]) {
        for ev in events {
            self.handle_event(ev);
        }
        self.update_welcome();
        self.env.touch();
    }

    fn handle_event(&mut self, ev: &IrcEvent) {
        match ev {
            IrcEvent::ConnAdded { conn, server, parent, netid } => {
                if !self.conns.contains_key(conn) {
                    let c = Conn::new(&self.env, conn, server, parent.clone(), netid.clone());
                    self.conns.insert(conn.clone(), c);
                    self.refresh_names();
                }
            }
            IrcEvent::ConnRemoved { conn } => self.remove_conn(conn),
            IrcEvent::Status { conn, .. }
            | IrcEvent::Registered { conn, .. }
            | IrcEvent::Caps { conn, .. }
            | IrcEvent::Isupport { conn, .. }
            | IrcEvent::Message { conn, .. } => {
                let ev = ev.clone();
                self.with_conn(conn, |app, c| app.handle_conn_event(c, ev));
            }
        }
    }

    fn handle_conn_event(&mut self, conn: &mut Conn, ev: IrcEvent) {
        match ev {
            IrcEvent::Status { status, error, retry_in, .. } => {
                let was = conn.status;
                conn.status = status;
                conn.error = error.clone();
                conn.retry_at = match retry_in {
                    Some(s) if s > 0 => self.now() + s as i64 * 1000,
                    _ => 0,
                };
                if status == ConnStatus::Disconnected {
                    for b in conn.buffers_iter_mut() {
                        b.joined = false;
                        b.clear_members();
                        b.typing.clear();
                        b.touch();
                    }
                    conn.online.clear();
                    conn.metadata.clear();
                    if was == ConnStatus::Connected || error.is_some() {
                        let mut text = String::from("Disconnected");
                        if let Some(e) = &error {
                            text += &format!(": {e}");
                        }
                        if let Some(s) = retry_in.filter(|s| *s > 0) {
                            text += &format!(" — retrying in {s}s");
                        }
                        let mut line = self.env.line(LineType::Error);
                        line.text = text;
                        conn.server.push(line);
                    }
                }
                if status == ConnStatus::Connected {
                    self.on_connected(conn);
                }
                conn.server.touch();
            }
            IrcEvent::Registered { nick, caps, cap_values, .. } => {
                conn.nick = nick;
                conn.caps = caps.into_iter().collect();
                conn.cap_values = cap_values;
            }
            IrcEvent::Caps { caps, cap_values, .. } => {
                conn.caps = caps.into_iter().collect();
                conn.cap_values = cap_values;
            }
            IrcEvent::Isupport { isupport, .. } => {
                conn.set_isupport(isupport);
                self.refresh_name_in(conn);
            }
            IrcEvent::Message { msg, .. } => {
                let bouncer = msg.command == "BOUNCER";
                crate::incoming::handle_message(self, conn, &msg);
                if bouncer {
                    self.refresh_name_in(conn);
                    let cid = conn.id.clone();
                    let kids: Vec<String> = self.conns.values().filter(|c| c.parent_id.as_deref() == Some(cid.as_str())).map(|c| c.id.clone()).collect();
                    for k in kids {
                        let name = self.compute_name_with_parent(&k, conn);
                        if let Some(c) = self.conns.get_mut(&k) {
                            c.set_name(name.clone());
                            if let Some(m) = self.meta.get_mut(&k) {
                                m.name = name;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        self.env.touch();
    }

    /// A child's name while its parent (which holds the bouncer's network list) is borrowed out.
    fn compute_name_with_parent(&self, child: &str, parent: &Conn) -> String {
        let c = &self.conns[child];
        if let Some(name) = c.netid.as_ref().and_then(|n| parent.networks.get(n)).and_then(|a| a.get("name")).filter(|n| !n.is_empty()) {
            return name.clone();
        }
        self.compute_name(c)
    }

    fn on_connected(&mut self, conn: &mut Conn) {
        conn.error = None;
        conn.auto_away = false;
        self.maybe_restore_active(conn, "");
        if self.active.is_none() {
            self.open_in(conn, "");
        }
        if conn.is_control() {
            return;
        }
        if !conn.buffers_loaded {
            crate::saved::restore_buffers(self, conn);
        } else {
            self.finish_connected(conn);
        }
    }

    /// The part of connecting that runs once saved buffers are back.
    pub(crate) fn finish_connected(&mut self, conn: &mut Conn) {
        if conn.has("draft/read-marker") {
            // Reconnecting: queries already exist, so ensure() won't ask for their markers.
            let names: Vec<String> = conn.buffers_iter().filter(|b| b.kind == BufferKind::Query).map(|b| b.name.clone()).collect();
            for n in names {
                conn.send(format!("MARKREAD {n}"));
            }
        }
        // Rejoin channels on direct connections; bouncers restore them on their own.
        if conn.netid.is_none() {
            let cfg = self.servers.iter().find(|s| s.id == conn.server_id);
            if !crate::saved::rejoin_lines(conn, cfg).is_empty() {
                self.env.schedule(1500, Timer::Rejoin { conn: conn.id.clone() });
            }
        }
        crate::monitor::sync_monitor(self, conn);
        self.env.schedule(800, Timer::FetchTargets { conn: conn.id.clone() });
    }

    /// Where replies and errors go: the buffer that sent the command (by
    /// label), else the active buffer if it's on this connection, else the
    /// server buffer. Returns the buffer's key.
    pub(crate) fn here_in(&self, conn: &Conn, label: Option<&str>) -> String {
        if let Some(k) = label.and_then(|l| conn.label_target(l)).filter(|k| conn.has_buffer_key(k)) {
            return k.clone();
        }
        match &self.active {
            Some(a) if a.conn == conn.id => a.key.clone(),
            _ => String::new(),
        }
    }

    /// Open `id` and scroll to a message, loading history around it if needed.
    pub fn jump_to(&mut self, id: &BufId, time: i64, msgid: Option<&str>) {
        self.open(id);
        crate::history::jump_to(self, id, time, msgid.map(String::from));
    }

    // ── Misc UI state ────────────────────────────────────────

    pub fn open_settings(&mut self, tab: &str, server_id: Option<&str>) {
        self.settings_open = Some(SettingsView { tab: tab.to_string(), server_id: server_id.map(String::from) });
        self.env.touch();
    }

    pub fn close_settings(&mut self) {
        self.settings_open = None;
        self.env.touch();
    }

    pub fn toggle_shortcuts(&mut self) {
        self.shortcuts_open = !self.shortcuts_open;
        self.env.touch();
    }

    pub(crate) fn update_welcome(&mut self) {
        if self.ready && self.servers.is_empty() {
            self.onboarding = true;
        }
        crate::welcome::update(self);
    }
}

/// Counts shown beside a buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BufSummary {
    pub unread: usize,
    pub highlights: usize,
    /// `None` means "default".
    pub level: Option<NotifyLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendKind {
    Privmsg,
    Notice,
}

impl SendKind {
    pub fn command(self) -> &'static str {
        match self {
            SendKind::Privmsg => "PRIVMSG",
            SendKind::Notice => "NOTICE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypingState {
    Active,
    Done,
}
