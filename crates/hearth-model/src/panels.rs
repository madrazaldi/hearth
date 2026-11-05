//! The logic behind the app's panels: quick switcher, channel browser,
//! sidebar tree, member list, and the settings pages' forms.

use crate::buffer::{collate, BufId, BufferKind, Member};
use crate::conn::{ChannelInfo, Conn};
use crate::format::{matching_rule, rule_error, strip_formatting};
use crate::ignore::is_ignored;
use crate::irc::{casefold, escape_attr};
use crate::state::{App, ToastKind};
use crate::types::{ConnStatus, NotifyLevel, SaslMechanism, ServerConfig, Source};
use hearth_core::certs::CertInfo;
use std::collections::BTreeMap;

// ── Quick switcher ────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Switcher {
    pub query: String,
    /// Highlighted result.
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitcherItem {
    Buffer {
        id: BufId,
        label: String,
        /** "network", "bouncer" or the network's name. */
        place: String,
        kind: BufferKind,
        unread: usize,
        highlights: usize,
        muted: bool,
        score: i32,
    },
    /// Join the channel just typed.
    Join { channel: String },
}

/// How well `name` matches `query`: 0 for no match.
pub fn switcher_score(name: &str, query: &str) -> i32 {
    if query.is_empty() {
        return 1;
    }
    let n = name.to_lowercase();
    let s = query.to_lowercase();
    let len = n.chars().count() as i32;
    if n == s {
        return 1000;
    }
    if n.trim_start_matches('#').starts_with(s.trim_start_matches('#')) {
        return 500 - len;
    }
    if let Some(i) = n.find(&s) {
        return 300 - n[..i].chars().count() as i32;
    }
    // Subsequence match, rewarding tight runs.
    let chars: Vec<char> = n.chars().collect();
    let (mut pos, mut gaps) = (0usize, 0i32);
    for ch in s.chars() {
        let Some(at) = chars[pos..].iter().position(|c| *c == ch) else { return 0 };
        gaps += at as i32;
        pos += at + 1;
    }
    100 - gaps
}

impl App {
    pub fn toggle_switcher(&mut self) {
        self.switcher = if self.switcher.is_some() { None } else { Some(Switcher::default()) };
        self.env.touch();
    }

    pub fn close_switcher(&mut self) {
        self.switcher = None;
        self.env.touch();
    }

    pub fn switcher_set_query(&mut self, q: &str) {
        if let Some(s) = self.switcher.as_mut() {
            s.query = q.to_string();
            s.selected = 0;
            self.env.touch();
        }
    }

    pub fn switcher_move(&mut self, delta: i32) {
        let n = self.switcher_results().len() as i32;
        if let (Some(s), true) = (self.switcher.as_mut(), n > 0) {
            s.selected = (s.selected as i32 + delta).rem_euclid(n) as usize;
            self.env.touch();
        }
    }

    /// Ranked matches for the switcher's query: at most 40, buffers by score, a join offer last.
    pub fn switcher_results(&self) -> Vec<SwitcherItem> {
        let q = self.switcher.as_ref().map(|s| s.query.trim().to_string()).unwrap_or_default();
        let mut out: Vec<SwitcherItem> = Vec::new();
        for id in self.all_buffer_ids() {
            let (Some(c), Some(b)) = (self.conns.get(&id.conn), self.buffer(&id)) else { continue };
            let server = b.kind == BufferKind::Server;
            let label = if server { c.name().to_string() } else { b.name.clone() };
            let sc = switcher_score(&label, &q);
            if sc <= 0 {
                continue;
            }
            let sum = self.summary(&id);
            let score =
                sc + if sum.highlights > 0 { 50 } else { 0 } + if sum.unread > 0 { 10 } else { 0 } + if self.active.as_ref() == Some(&id) { -40 } else { 0 };
            let place = if server { (if c.is_control() { "bouncer" } else { "network" }).to_string() } else { c.name().to_string() };
            out.push(SwitcherItem::Buffer {
                id,
                label,
                place,
                kind: b.kind,
                unread: sum.unread,
                highlights: sum.highlights,
                muted: sum.level == Some(NotifyLevel::None),
                score,
            });
        }
        out.sort_by_key(|i| match i {
            SwitcherItem::Buffer { score, .. } => std::cmp::Reverse(*score),
            SwitcherItem::Join { .. } => std::cmp::Reverse(i32::MIN),
        });
        if let Some(c) = self.active_conn() {
            if !q.is_empty() && c.is_channel(&q) && c.buffer(&q).is_none() {
                out.push(SwitcherItem::Join { channel: q });
            }
        }
        out.truncate(40);
        out
    }

    /// Open the highlighted result (or the one given) and close the switcher.
    pub fn switcher_choose(&mut self, index: Option<usize>) {
        let results = self.switcher_results();
        let i = index.or(self.switcher.as_ref().map(|s| s.selected)).unwrap_or(0);
        self.switcher = None;
        match results.get(i) {
            Some(SwitcherItem::Buffer { id, .. }) => self.open(id),
            Some(SwitcherItem::Join { channel }) => {
                if let Some(a) = self.active.clone() {
                    self.join(&a.conn, channel, None);
                }
            }
            None => {}
        }
        self.env.touch();
    }
}

// ── Channel browser ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelSort {
    #[default]
    Users,
    Name,
    Topic,
}

#[derive(Debug, Clone)]
pub struct BrowserState {
    pub query: String,
    pub sort: ChannelSort,
    /// Largest, last or Z to A first. Users start descending (the busiest channels first), the others ascending.
    pub descending: bool,
}

impl Default for BrowserState {
    fn default() -> Self {
        BrowserState { query: String::new(), sort: ChannelSort::Users, descending: true }
    }
}

/// How many channels the browser lists before "refine your search".
pub const BROWSER_SHOWN: usize = 300;

#[derive(Debug, Clone)]
pub struct BrowserView {
    pub loading: bool,
    /// The list has been fetched (or is being).
    pub has_list: bool,
    /// Channels matching, up to [`BROWSER_SHOWN`].
    pub items: Vec<ChannelInfo>,
    /// Matches in all, before the cut.
    pub total: usize,
}

impl App {
    /// The channel list for the open browser, filtered and sorted.
    pub fn browser_view(&self) -> Option<BrowserView> {
        let conn = self.conns.get(self.browser_conn.as_ref()?)?;
        let list = conn.channel_list.as_ref();
        let needle = self.browser.query.trim().to_lowercase();
        let mut items: Vec<ChannelInfo> = list
            .map(|l| &l.items[..])
            .unwrap_or(&[])
            .iter()
            .filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle) || strip_formatting(&c.topic).to_lowercase().contains(&needle))
            .cloned()
            .collect();
        let desc = self.browser.descending;
        let flip = |o: std::cmp::Ordering| if desc { o.reverse() } else { o };
        // Ties always read A to Z, so flipping the direction moves whole groups, not their insides.
        let by_name = |a: &ChannelInfo, b: &ChannelInfo| collate(&conn.fold(&a.name), &conn.fold(&b.name));
        match self.browser.sort {
            ChannelSort::Users => items.sort_by(|a, b| flip(a.users.cmp(&b.users)).then_with(|| by_name(a, b))),
            ChannelSort::Name => items.sort_by(|a, b| flip(by_name(a, b))),
            ChannelSort::Topic => {
                let topic = |c: &ChannelInfo| strip_formatting(&c.topic).trim().to_lowercase();
                // Channels without a topic stay at the bottom either way.
                items.sort_by(|a, b| {
                    let (ta, tb) = (topic(a), topic(b));
                    match (ta.is_empty(), tb.is_empty()) {
                        (true, true) => by_name(a, b),
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        (false, false) => flip(collate(&ta, &tb)).then_with(|| by_name(a, b)),
                    }
                })
            }
        }
        let total = items.len();
        items.truncate(BROWSER_SHOWN);
        Some(BrowserView { loading: list.is_some_and(|l| l.loading), has_list: list.is_some(), items, total })
    }

    pub fn browser_set_query(&mut self, q: &str) {
        self.browser.query = q.to_string();
        self.env.touch();
    }

    /// Sort by a column: choosing the one already used reverses it.
    pub fn browser_set_sort(&mut self, sort: ChannelSort) {
        if self.browser.sort == sort {
            self.browser.descending = !self.browser.descending;
        } else {
            self.browser.sort = sort;
            self.browser.descending = sort == ChannelSort::Users;
        }
        self.env.touch();
    }

    /// Join (or open) a channel from the browser and close it.
    pub fn browser_join(&mut self, name: &str) {
        if let Some(cid) = self.browser_conn.clone() {
            self.join(&cid, name, None);
        }
        self.close_channel_browser();
    }
}

// ── Sidebar ───────────────────────────────────────────────────

/// The notification levels a buffer's menu offers, with their labels.
pub const NOTIFY_LEVELS: &[(Option<NotifyLevel>, &str)] =
    &[(None, "Default"), (Some(NotifyLevel::All), "All messages"), (Some(NotifyLevel::Mentions), "Mentions only"), (Some(NotifyLevel::None), "Nothing (mute)")];

/// One row of the sidebar tree: a network, or a channel or conversation under one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarNode {
    /// Stable key for keyboard focus.
    pub id: String,
    pub conn: String,
    /// `None` for a network row.
    pub buf: Option<BufId>,
    /// The network's name, or the buffer's name without its `#`.
    pub name: String,
    pub kind: BufferKind,
    /// The network's connection status (network rows).
    pub status: ConnStatus,
    /// "", "offline", "connecting", "retrying in 4s" (network rows).
    pub status_label: String,
    pub is_control: bool,
    pub collapsed: bool,
    /// The connection is marked away (network rows).
    pub away: bool,
    pub active: bool,
    pub unread: usize,
    pub highlights: usize,
    pub level: Option<NotifyLevel>,
    pub joined: bool,
    /// A query's MONITOR state, when known.
    pub online: Option<bool>,
    /// Someone is typing in this conversation.
    pub typing: bool,
    /// For screen readers.
    pub label: String,
    /// The first letter, for a network tile.
    pub initial: String,
}

pub fn status_label(c: &Conn, now: i64) -> String {
    match c.status {
        ConnStatus::Connected => String::new(),
        ConnStatus::Disconnected if c.retry_at > 0 => format!("retrying in {}s", ((c.retry_at - now) as f64 / 1000.0).round().max(0.0) as i64),
        ConnStatus::Disconnected => "offline".into(),
        _ => "connecting".into(),
    }
}

/// "Online", "Away", "Offline" or "Connecting\u{2026}" for a network's presence line.
pub fn presence(c: &Conn) -> &'static str {
    match c.status {
        ConnStatus::Connected if c.away => "Away",
        ConnStatus::Connected => "Online",
        ConnStatus::Disconnected => "Offline",
        _ => "Connecting\u{2026}",
    }
}

impl App {
    /// The sidebar, top to bottom. A collapsed network hides its buffers (a bouncer's control connection never collapses).
    pub fn sidebar(&self) -> Vec<SidebarNode> {
        let now = self.now();
        let mut out = Vec::new();
        for c in self.ordered_conns() {
            let net_id = BufId::server(&c.id);
            let sum = self.summary(&net_id);
            let status = status_label(c, now);
            let hl_total: usize = c.channel_buffers().iter().map(|b| self.summary(&b.id()).highlights).sum();
            let mut parts = vec![c.name().to_string(), (if c.is_control() { "bouncer" } else { "network" }).to_string()];
            if !status.is_empty() {
                parts.push(status.clone());
            }
            if c.collapsed && !c.is_control() && hl_total > 0 {
                parts.push(format!("{hl_total} mention{}", if hl_total == 1 { "" } else { "s" }));
            }
            out.push(SidebarNode {
                id: format!("n{}", c.id),
                conn: c.id.clone(),
                buf: None,
                name: c.name().to_string(),
                kind: BufferKind::Server,
                status: c.status,
                status_label: status,
                is_control: c.is_control(),
                collapsed: c.collapsed,
                away: c.away,
                active: self.active.as_ref() == Some(&net_id),
                unread: sum.unread,
                highlights: sum.highlights,
                level: sum.level,
                joined: true,
                online: None,
                typing: false,
                label: parts.join(", "),
                initial: crate::rows::initial(c.name()),
            });
            if !c.is_control() && c.collapsed {
                continue;
            }
            for b in c.channel_buffers() {
                let id = b.id();
                let sum = self.summary(&id);
                let online = (b.kind == BufferKind::Query).then(|| c.online.get(&c.fold(&b.name)).copied()).flatten();
                let typing = b.kind == BufferKind::Query && !b.typing.is_empty();
                out.push(SidebarNode {
                    id: format!("b{}\u{0}{}", id.conn, id.key),
                    conn: c.id.clone(),
                    name: if b.kind == BufferKind::Channel { b.name.trim_start_matches('#').to_string() } else { b.name.clone() },
                    kind: b.kind,
                    status: c.status,
                    status_label: String::new(),
                    is_control: false,
                    collapsed: false,
                    away: false,
                    active: self.active.as_ref() == Some(&id),
                    unread: sum.unread,
                    highlights: sum.highlights,
                    level: sum.level,
                    joined: b.joined,
                    online,
                    typing,
                    label: crate::a11y::buffer_label(&crate::a11y::BufferLabel {
                        name: &b.name,
                        kind: b.kind,
                        unread: sum.unread,
                        highlights: sum.highlights,
                        muted: sum.level == Some(NotifyLevel::None),
                        joined: Some(b.joined),
                        online,
                        typing,
                    }),
                    initial: String::new(),
                    buf: Some(id),
                });
            }
        }
        out
    }

    pub fn toggle_collapsed(&mut self, conn: &str) {
        if let Some(c) = self.conns.get_mut(conn) {
            c.collapsed = !c.collapsed;
            self.env.touch();
        }
    }

    pub fn set_collapsed(&mut self, conn: &str, collapsed: bool) {
        if let Some(c) = self.conns.get_mut(conn) {
            c.collapsed = collapsed;
            self.env.touch();
        }
    }

    /// Resize the sidebar, within 190 to 420 pixels.
    pub fn set_sidebar_width(&mut self, w: i32) {
        self.update_settings(|s| s.sidebar_width = w.clamp(190, 420) as u32);
    }

    pub fn toggle_members(&mut self) {
        self.update_settings(|s| s.show_members = !s.show_members);
    }
}

// ── Member list ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberRow {
    Head {
        title: String,
        count: usize,
    },
    Member {
        member: Member,
        /** Position among members, from 1. */
        pos: usize,
        label: String,
    },
}

fn rank_title(mode: Option<char>, has_prefix: bool) -> String {
    if !has_prefix {
        return "Members".into();
    }
    match mode {
        Some('q') => "Owners".into(),
        Some('a') => "Admins".into(),
        Some('o') => "Operators".into(),
        Some('h') => "Half-operators".into(),
        Some('v') => "Voiced".into(),
        Some(m) => format!("Mode {m}"),
        None => "Mode ".into(),
    }
}

impl App {
    /// Members grouped by rank, as the side panel lists them. `query` narrows by nick.
    pub fn member_rows(&self, id: &BufId, query: &str) -> Vec<MemberRow> {
        let (Some(c), Some(b)) = (self.conns.get(&id.conn), self.buffer(id)) else { return Vec::new() };
        let title_of = |m: &Member| rank_title(m.top_prefix().and_then(|s| c.prefix().mode_of(s)), m.top_prefix().is_some());
        let q = query.trim().to_lowercase();
        let all = b.sorted_members(c.prefix());
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for m in &all {
            *counts.entry(title_of(m)).or_default() += 1;
        }
        let (mut rows, mut title, mut pos) = (Vec::new(), String::new(), 0);
        for m in all {
            if !q.is_empty() && !m.nick.to_lowercase().contains(&q) {
                continue;
            }
            let t = title_of(m);
            if t != title {
                rows.push(MemberRow::Head { count: counts[&t], title: t.clone() });
                title = t;
            }
            pos += 1;
            let dn = c.profile(&m.nick).display_name;
            rows.push(MemberRow::Member { label: crate::a11y::member_label(m, c.prefix(), dn.as_deref()), member: m.clone(), pos });
        }
        rows
    }
}

// ── Nick menu ─────────────────────────────────────────────────

/// What the menu on a nick offers, given who is asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NickMenu {
    pub nick: String,
    pub member: Option<Member>,
    pub is_self: bool,
    /// We hold at least operator rank here.
    pub can_moderate: bool,
    pub is_op: bool,
    pub is_voice: bool,
    pub ignored: bool,
    pub watched: bool,
    /// MONITOR state: `Some(true)` online.
    pub online: Option<bool>,
    pub avatar: Option<String>,
    pub display_name: Option<String>,
    pub can_watch: bool,
}

impl App {
    pub fn nick_menu(&self, id: &BufId, nick: &str) -> Option<NickMenu> {
        let c = self.conns.get(&id.conn)?;
        let b = c.buf(&id.key)?;
        let member = b.member(nick).cloned();
        let my_rank = b.member(&c.nick).and_then(|m| m.top_prefix()).and_then(|s| c.prefix().rank(s));
        let op_rank = c.prefix().rank('@').unwrap_or(0);
        let src = Source { nick: nick.to_string(), user: member.as_ref().and_then(|m| m.user.clone()), host: member.as_ref().and_then(|m| m.host.clone()) };
        let profile = c.profile(nick);
        Some(NickMenu {
            nick: nick.to_string(),
            is_op: member.as_ref().is_some_and(|m| m.prefix.contains('@')),
            is_voice: member.as_ref().is_some_and(|m| m.prefix.contains('+')),
            member,
            is_self: c.is_me(nick),
            can_moderate: b.kind == BufferKind::Channel && my_rank.is_some_and(|r| r <= op_rank),
            ignored: self.is_ignored(c, Some(&src)),
            watched: crate::monitor::is_watched(self, c, nick),
            online: c.online.get(&c.fold(nick)).copied(),
            avatar: profile.avatar.filter(|_| self.settings.load_avatars),
            display_name: profile.display_name,
            can_watch: c.can_monitor(),
        })
    }

    pub fn watch(&mut self, conn: &str, nick: &str) {
        let n = nick.to_string();
        self.with_conn(conn, |app, c| crate::monitor::watch(app, c, &n));
    }

    pub fn unwatch(&mut self, conn: &str, nick: &str) {
        let n = nick.to_string();
        self.with_conn(conn, |app, c| crate::monitor::unwatch(app, c, &n));
    }
}

// ── Settings ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsView {
    pub tab: String,
    /// `"new"` for the add-server form, a server id to edit it.
    pub server_id: Option<String>,
}

/// Settings tabs: id, label, icon name.
pub const SETTINGS_TABS: &[(&str, &str, &str)] = &[
    ("servers", "Servers", "server"),
    ("general", "General", "gear"),
    ("appearance", "Appearance", "moon"),
    ("notifications", "Notifications", "bell"),
    ("aliases", "Aliases", "terminal"),
    ("privacy", "Privacy & data", "lock"),
    ("shortcuts", "Shortcuts", "keyboard"),
];

pub fn settings_description(tab: &str) -> &'static str {
    match tab {
        "servers" => "Networks and bouncers Hearth connects to. A soju bouncer expands into all of its networks automatically.",
        "general" => "Startup behavior, presence and updates.",
        "appearance" => "Theme, message layout and how the conversation is displayed.",
        "notifications" => "Choose what gets your attention.",
        "privacy" => "What Hearth stores on this computer and what it fetches on your behalf.",
        "shortcuts" => "Keyboard shortcuts available throughout Hearth.",
        _ => "",
    }
}

impl App {
    /// The server the settings page is editing: a blank one for "new".
    pub fn settings_edit_server(&self) -> Option<ServerConfig> {
        let v = self.settings_open.as_ref()?;
        match v.server_id.as_deref()? {
            "new" => Some(crate::welcome::default_server()),
            id => self.servers.iter().find(|s| s.id == id).cloned(),
        }
    }

    /// The settings page's heading.
    pub fn settings_title(&self) -> String {
        let Some(v) = &self.settings_open else { return "Settings".into() };
        if v.tab == "servers" {
            if let Some(s) = self.settings_edit_server() {
                return if v.server_id.as_deref() == Some("new") { "Add a server".into() } else { format!("Edit {}", s.name) };
            }
        }
        SETTINGS_TABS.iter().find(|t| t.0 == v.tab).map_or("Settings", |t| t.1).to_string()
    }

    /// The live connection of a server, when the page is editing a bouncer.
    pub fn settings_bouncer(&self) -> Option<&Conn> {
        let id = self.settings_open.as_ref()?.server_id.as_deref().filter(|i| *i != "new")?;
        self.conns.get(id)
    }

    /// A server's status for the servers list.
    pub fn server_status(&self, id: &str) -> &'static str {
        match self.conns.get(id).map(|c| c.status) {
            None => "offline",
            Some(ConnStatus::Connected) => "connected",
            Some(ConnStatus::Connecting) => "connecting",
            Some(ConnStatus::Registering) => "registering",
            Some(ConnStatus::Disconnected) => "disconnected",
        }
    }

    /// Set the highlight words from a comma-separated list.
    pub fn set_highlight_words(&mut self, text: &str) {
        let words: Vec<String> = text.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
        self.update_settings(|s| s.highlight_words = words);
    }

    /// Add an ignore mask from the settings field.
    pub fn add_ignore(&mut self, text: &str) {
        if !text.trim().is_empty() {
            self.ignore(text);
        }
    }

    pub fn remove_ignore(&mut self, mask: &str) {
        let next: Vec<String> = self.settings.ignores.iter().filter(|m| *m != mask).cloned().collect();
        self.update_settings(|s| s.ignores = next);
    }
}

// ── Server form ───────────────────────────────────────────────

/// The add/edit server form's state and rules.
#[derive(Debug)]
pub struct ServerForm {
    pub server: ServerConfig,
    /// Comma- or space-separated channels.
    pub autojoin: String,
    /// One raw command per line.
    pub perform: String,
    pub error: String,
    pub saving: bool,
    pub advanced: bool,
    pub confirm_delete: bool,
    pub cert: Option<CertInfo>,
    pub cert_error: String,
}

impl ServerForm {
    pub fn new(server: ServerConfig) -> ServerForm {
        ServerForm {
            autojoin: server.autojoin.join(", "),
            perform: server.perform.join("\n"),
            server,
            error: String::new(),
            saving: false,
            advanced: false,
            confirm_delete: false,
            cert: None,
            cert_error: String::new(),
        }
    }

    /// `"ws"` or `"tcp"`.
    pub fn transport(&self) -> &'static str {
        if self.server.websocket {
            "ws"
        } else {
            "tcp"
        }
    }

    /// Switch between TCP and WebSocket, moving the port off the other transport's default.
    pub fn set_transport(&mut self, ws: bool) {
        let s = &mut self.server;
        s.websocket = ws;
        if ws && (s.port == 6697 || s.port == 6667) {
            s.port = if s.tls { 443 } else { 80 };
        }
        if !ws && (s.port == 443 || s.port == 80) {
            s.port = if s.tls { 6697 } else { 6667 };
        }
    }

    pub fn set_tls(&mut self, tls: bool) {
        let s = &mut self.server;
        s.tls = tls;
        if !s.websocket && s.port == if tls { 6667 } else { 6697 } {
            s.port = if tls { 6697 } else { 6667 };
        }
        if s.websocket && s.port == if tls { 80 } else { 443 } {
            s.port = if tls { 443 } else { 80 };
        }
    }

    /// Validate and normalise; the server to save, or the message to show.
    pub fn submit(&mut self) -> Result<ServerConfig, String> {
        self.error.clear();
        let fail = |this: &mut ServerForm, m: &str| {
            this.error = m.to_string();
            Err(m.to_string())
        };
        if self.server.host.trim().is_empty() {
            return fail(self, "A host is required.");
        }
        if self.server.nick.trim().is_empty() {
            return fail(self, "Choose a nickname.");
        }
        self.server.autojoin = self
            .autojoin
            .split(|c: char| c.is_whitespace() || c == ',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| if c.starts_with(['#', '&', '+', '!']) { c.to_string() } else { format!("#{c}") })
            .collect();
        self.server.perform = self.perform.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
        if self.server.sasl == SaslMechanism::Plain && self.server.sasl_username.is_empty() {
            self.server.sasl_username = self.server.nick.clone();
        }
        if self.server.sasl == SaslMechanism::External && self.server.client_cert.trim().is_empty() {
            return fail(self, "Choose or generate a client certificate.");
        }
        if self.server.sasl == SaslMechanism::External && !self.server.tls {
            return fail(self, "Certificate authentication needs TLS.");
        }
        Ok(self.server.clone())
    }
}

// ── Bouncer networks ──────────────────────────────────────────

/// The add/edit form for a soju network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkDraft {
    /// `None` when adding.
    pub netid: Option<String>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub tls: bool,
    pub nickname: String,
    pub username: String,
    pub pass: String,
}

impl Default for NetworkDraft {
    fn default() -> Self {
        NetworkDraft {
            netid: None,
            name: String::new(),
            host: String::new(),
            port: "6697".into(),
            tls: true,
            nickname: String::new(),
            username: String::new(),
            pass: String::new(),
        }
    }
}

impl NetworkDraft {
    pub fn edit(netid: &str, attrs: &BTreeMap<String, String>) -> NetworkDraft {
        let get = |k: &str| attrs.get(k).cloned().unwrap_or_default();
        NetworkDraft {
            netid: Some(netid.into()),
            name: get("name"),
            host: get("host"),
            port: get("port"),
            tls: attrs.get("tls").map(String::as_str) != Some("0"),
            nickname: get("nickname"),
            username: get("username"),
            pass: String::new(),
        }
    }

    /// The `;`-joined attributes to send. With `original`, only what changed.
    pub fn attrs(&self, original: Option<&BTreeMap<String, String>>) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut put = |k: &str, v: &str| {
            let changed = match original {
                Some(o) => o.get(k).map(String::as_str).unwrap_or("") != v,
                None => !v.is_empty(),
            };
            if changed {
                out.push(format!("{k}={}", escape_attr(v)));
            }
        };
        put("name", self.name.trim());
        put("host", self.host.trim());
        if !self.port.trim().is_empty() {
            put("port", self.port.trim());
        }
        put("tls", if self.tls { "1" } else { "0" });
        put("nickname", self.nickname.trim());
        put("username", self.username.trim());
        if !self.pass.is_empty() {
            out.push(format!("pass={}", escape_attr(&self.pass)));
        }
        out.join(";")
    }
}

impl App {
    /// A bouncer's networks, by name.
    pub fn bouncer_networks(&self, conn: &str) -> Vec<(String, BTreeMap<String, String>)> {
        let Some(c) = self.conns.get(conn) else { return Vec::new() };
        let mut v: Vec<_> = c.networks.iter().map(|(k, a)| (k.clone(), a.clone())).collect();
        v.sort_by(|a, b| collate(a.1.get("name").unwrap_or(&a.0), b.1.get("name").unwrap_or(&b.0)));
        v
    }

    /// Send the draft to the bouncer. False (with a toast) if it can't work.
    pub fn bouncer_save(&mut self, conn: &str, d: &NetworkDraft) -> bool {
        if d.host.trim().is_empty() {
            self.toast("A host is required", ToastKind::Error);
            return false;
        }
        let Some(c) = self.conns.get(conn) else { return false };
        match &d.netid {
            Some(id) => {
                let changes = d.attrs(c.networks.get(id));
                if !changes.is_empty() {
                    c.send(format!("BOUNCER CHANGENETWORK {id} {changes}"));
                }
            }
            None => c.send(format!("BOUNCER ADDNETWORK {}", d.attrs(None))),
        }
        true
    }

    pub fn bouncer_delete(&mut self, conn: &str, netid: &str) {
        if let Some(c) = self.conns.get(conn) {
            c.send(format!("BOUNCER DELNETWORK {netid}"));
        }
    }
}

// ── Highlight rules ───────────────────────────────────────────

/// Per-network highlight words and exceptions, with a tester.
#[derive(Debug, Clone, Default)]
pub struct HighlightsForm {
    /// One word or `/regex/flags` per line.
    pub words: String,
    /// Nicks or masks, comma- or space-separated.
    pub never: String,
    pub sample: String,
    pub sample_from: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightTest {
    pub hit: bool,
    pub why: String,
}

impl HighlightsForm {
    pub fn for_server(app: &App, server_id: &str) -> HighlightsForm {
        let n = app.network_highlights(server_id);
        HighlightsForm { words: n.words.join("\n"), never: n.never.join(", "), ..Default::default() }
    }

    pub fn lines(&self) -> Vec<String> {
        self.words.lines().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect()
    }

    pub fn errors(&self) -> Vec<String> {
        self.lines().iter().filter_map(|w| rule_error(w).map(|e| format!("{w} \u{2014} {e}"))).collect()
    }

    pub fn valid(&self) -> Vec<String> {
        self.lines().into_iter().filter(|w| rule_error(w).is_none()).collect()
    }

    pub fn never_list(&self) -> Vec<String> {
        self.never.split(|c: char| c.is_whitespace() || c == ',').map(str::trim).filter(|n| !n.is_empty()).map(String::from).collect()
    }
}

impl App {
    /// Would `form.sample` from `form.sample_from` highlight on this server?
    pub fn highlight_test(&self, server_id: &str, nick: &str, form: &HighlightsForm) -> Option<HighlightTest> {
        if form.sample.trim().is_empty() {
            return None;
        }
        let conn = self.conns.values().find(|c| c.server_id == server_id && !c.is_control());
        let my_nick = conn.map(|c| c.nick.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| nick.to_string());
        let fold = |s: &str| conn.map_or_else(|| casefold(s, "rfc1459"), |c| c.fold(s));
        let from = form.sample_from.trim();
        if !from.is_empty() && is_ignored(&form.never_list(), &Source { nick: from.into(), user: None, host: None }, fold) {
            return Some(HighlightTest { hit: false, why: format!("{from} never highlights") });
        }
        let rules: Vec<String> =
            std::iter::once(my_nick.clone()).chain(self.settings.highlight_words.iter().cloned()).chain(form.valid()).filter(|r| !r.is_empty()).collect();
        Some(match matching_rule(&form.sample, &rules) {
            None => HighlightTest { hit: false, why: "No rule matches".into() },
            Some(r) if *r == my_nick => HighlightTest { hit: true, why: "Matches your nickname".into() },
            Some(r) => HighlightTest { hit: true, why: format!("Matches {r}") },
        })
    }

    /// Save the form. Nothing changes while a rule is invalid, so a typo doesn't drop the rule.
    pub fn highlights_save(&mut self, server_id: &str, form: &HighlightsForm) {
        if !form.errors().is_empty() {
            return;
        }
        let (valid, never) = (form.valid(), form.never_list());
        let id = server_id.to_string();
        self.update_settings(|s| {
            if valid.is_empty() && never.is_empty() {
                s.network_highlights.remove(&id);
            } else {
                s.network_highlights.insert(id, crate::types::NetworkHighlights { words: valid, never });
            }
        });
    }
}

// ── Aliases ───────────────────────────────────────────────────

/// The alias editor's fields.
#[derive(Debug, Clone, Default)]
pub struct AliasForm {
    pub name: String,
    pub body: String,
    /// A save was tried, so errors show.
    pub tried: bool,
}

impl AliasForm {
    pub fn key(&self) -> String {
        crate::aliases::alias_name(&self.name)
    }

    pub fn error(&self) -> Option<&'static str> {
        crate::aliases::alias_error(&self.key(), &self.body)
    }

    pub fn name_bad(&self) -> bool {
        self.tried && self.error().is_some_and(|e| e != crate::aliases::BODY_ERROR)
    }

    pub fn body_bad(&self) -> bool {
        self.tried && self.error() == Some(crate::aliases::BODY_ERROR)
    }

    /// The name shadows a built-in command.
    pub fn shadows_builtin(&self) -> bool {
        crate::commands::is_builtin(&self.key())
    }
}

impl App {
    pub fn aliases_sorted(&self) -> Vec<(String, String)> {
        let mut v: Vec<_> = self.settings.aliases.iter().map(|(k, b)| (k.clone(), b.clone())).collect();
        v.sort_by(|a, b| collate(&a.0, &b.0));
        v
    }

    pub fn alias_exists(&self, form: &AliasForm) -> bool {
        self.settings.aliases.contains_key(&form.key())
    }

    /// Save the alias; false if the form has an error (which then shows).
    pub fn alias_save(&mut self, form: &mut AliasForm) -> bool {
        form.tried = true;
        if form.error().is_some() {
            return false;
        }
        let (key, body) = (form.key(), form.body.trim().to_string());
        self.update_settings(|s| {
            s.aliases.insert(key, body);
        });
        form.name.clear();
        form.body.clear();
        form.tried = false;
        true
    }

    pub fn alias_remove(&mut self, name: &str) {
        let n = name.to_string();
        self.update_settings(|s| {
            s.aliases.remove(&n);
        });
    }

    pub fn alias_edit(&self, form: &mut AliasForm, name: &str) {
        form.name = name.to_string();
        form.body = self.settings.aliases.get(name).cloned().unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_exact_prefix_substring_and_subsequence() {
        assert_eq!(switcher_score("anything", ""), 1);
        assert_eq!(switcher_score("#rust", "#rust"), 1000);
        assert_eq!(switcher_score("#rust", "ru"), 500 - 5);
        assert_eq!(switcher_score("#trust", "ru"), 300 - 2);
        assert!(switcher_score("#hearth-dev", "hdv") > 0);
        assert_eq!(switcher_score("#hearth", "zz"), 0);
    }

    #[test]
    fn server_form_moves_ports_with_the_transport() {
        let mut f = ServerForm::new(ServerConfig { host: "h".into(), nick: "n".into(), ..Default::default() });
        f.set_transport(true);
        assert_eq!(f.server.port, 443);
        f.set_tls(false);
        assert_eq!(f.server.port, 80);
        f.set_transport(false);
        assert_eq!(f.server.port, 6667);
        f.autojoin = "a, #b  c".into();
        let s = f.submit().unwrap();
        assert_eq!(s.autojoin, ["#a", "#b", "#c"]);
        f.server.sasl = SaslMechanism::External;
        assert!(f.submit().unwrap_err().contains("certificate"));
    }

    #[test]
    fn bouncer_drafts_send_only_what_changed() {
        let mut orig = BTreeMap::new();
        orig.insert("name".to_string(), "Libera".to_string());
        orig.insert("host".to_string(), "irc.libera.chat".to_string());
        let mut d = NetworkDraft::edit("1", &orig);
        d.host = "other host".into();
        assert!(d.attrs(Some(&orig)).contains("host=other\\shost"));
        assert!(!d.attrs(Some(&orig)).contains("name="));
        assert!(NetworkDraft { host: "h".into(), ..Default::default() }.attrs(None).starts_with("host=h;port=6697;tls=1"));
    }
}
