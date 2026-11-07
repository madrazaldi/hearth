//! The search panel: query box, scope, results, `from:`/`in:` suggestions
//! and the debounce that keeps typing from firing a search per key.

use crate::buffer::BufId;
use crate::env::Timer;
use crate::format::strip_formatting;
use crate::irc::parse_ctcp;
use crate::search::{dedupe_hits, hit_time, parse_query, SearchScope};
use crate::state::{App, ToastKind};
use crate::types::SearchHit;
use regex::Regex;
use std::sync::OnceLock;

/// How long typing must pause before a search starts.
const DEBOUNCE_MS: i64 = 180;
/// Asking for more results starts this many rows from the end.
const MORE_AHEAD: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelScope {
    #[default]
    All,
    /// The origin buffer's network.
    Network,
    /// The origin buffer only.
    Buffer,
}

/// A filter the panel offers as a chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterHint {
    pub prefix: &'static str,
    pub hint: &'static str,
    pub label: &'static str,
}

pub const FILTERS: &[FilterHint] = &[
    FilterHint { prefix: "from:", hint: "nick", label: "Messages from someone" },
    FilterHint { prefix: "in:", hint: "channel", label: "In one channel or conversation" },
    FilterHint { prefix: "on:", hint: "2026-10-12", label: "On a day; also today or yesterday" },
    FilterHint { prefix: "before:", hint: "date", label: "Before a day" },
    FilterHint { prefix: "after:", hint: "date", label: "After a day" },
];

/// Keys the panel's input handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKey {
    Down,
    Up,
    Enter,
    Tab,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    q: String,
    origin: Option<BufId>,
    scope: PanelScope,
    hits: Vec<SearchHit>,
    next: Option<i64>,
    sel: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SearchPanel {
    pub open: bool,
    /// The query. Kept after closing, so reopening restores it.
    pub q: String,
    pub scope: PanelScope,
    /// Where the panel was opened from, which scopes and suggestions refer to.
    pub origin: Option<BufId>,
    pub hits: Vec<SearchHit>,
    /// Pass back for older hits; `None` when exhausted.
    pub next: Option<i64>,
    pub busy: bool,
    /// Older hits are loading.
    pub more: bool,
    pub sel: usize,
    pub sug_sel: usize,
    /// Bumped by every change of query or scope, so late answers are ignored.
    run: u64,
    job: Option<u64>,
    more_job: Option<u64>,
    /// The last search that led to a jump, to pick up where it left off.
    last: Option<Snapshot>,
    /// The panel was reopened onto its last results: don't search again, and
    /// select the text so typing replaces it.
    pub resumed: bool,
}

/// A hit, laid out for a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitView {
    /// The buffer it belongs to.
    pub buffer: String,
    /// The network's name, when known.
    pub network: String,
    pub nick: String,
    /// Plain text (formatting codes and CTCP framing removed).
    pub text: String,
    /// `text` split into plain and matched parts.
    pub parts: Vec<(String, bool)>,
    pub time: i64,
    /// `time` formatted for display.
    pub stamp: String,
    /// The hit's network is known, so choosing it can jump there. Otherwise the row is dimmed and choosing it only explains.
    pub reachable: bool,
}

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(?:^|\s)(from|in):(\S*)$").unwrap())
}

impl SearchPanel {
    /// The `from:`/`in:` word being typed at the end of the query.
    fn token(&self) -> Option<(String, String)> {
        let c = token_re().captures(&self.q)?;
        Some((c[1].to_lowercase(), c[2].to_string()))
    }
}

impl App {
    /// Whether any search source exists: the local log, or a bouncer that searches.
    pub fn can_search(&self) -> bool {
        self.settings.store_logs || self.conns.values().any(|c| c.has("soju.im/search"))
    }

    /// Open the panel. If it was last closed by choosing a result for the
    /// same query, it comes back with those results.
    pub fn search_open(&mut self) {
        self.open_search_with(None);
    }

    /// Open the panel on a query (the `/search` command).
    pub fn open_search(&mut self, q: &str) {
        self.open_search_with(Some(q.trim().to_string()));
    }

    fn open_search_with(&mut self, q: Option<String>) {
        let active = self.active.clone();
        let p = &mut self.search_panel;
        if let Some(q) = q {
            p.q = q;
        }
        p.open = true;
        p.run += 1;
        p.sug_sel = 0;
        p.resumed = false;
        match p.last.clone().filter(|l| !p.q.is_empty() && l.q == p.q) {
            Some(l) => {
                p.origin = l.origin;
                p.scope = l.scope;
                p.hits = l.hits;
                p.next = l.next;
                p.sel = l.sel;
                p.busy = false;
                p.more = false;
                p.resumed = true;
            }
            None => {
                p.origin = active;
                p.scope = PanelScope::All;
                p.hits.clear();
                p.next = None;
                p.sel = 0;
                p.busy = false;
                p.more = false;
                p.last = None;
                let run = p.run;
                if !p.q.trim().is_empty() {
                    p.busy = true;
                    self.env.schedule(DEBOUNCE_MS, Timer::SearchDebounce { run });
                }
            }
        }
        self.abandon_jobs();
        self.env.touch();
    }

    pub fn search_close(&mut self) {
        self.abandon_jobs();
        self.env.cancel(&Timer::SearchDebounce { run: self.search_panel.run });
        self.search_panel.open = false;
        self.search_panel.busy = false;
        self.env.touch();
    }

    pub fn search_panel(&self) -> &SearchPanel {
        &self.search_panel
    }

    /// The origin buffer's network, when the panel has an origin.
    pub fn search_origin_name(&self) -> Option<(String, String)> {
        let id = self.search_panel.origin.as_ref()?;
        let c = self.conns.get(&id.conn)?;
        let b = self.buffer(id)?;
        Some((c.name().to_string(), if id.is_server() { c.name().to_string() } else { b.name.clone() }))
    }

    fn abandon_jobs(&mut self) {
        for j in [self.search_panel.job.take(), self.search_panel.more_job.take()].into_iter().flatten() {
            self.cancel_search(j);
        }
    }

    /// The text changed: search again after a pause.
    pub fn search_set_query(&mut self, q: &str) {
        if self.search_panel.q == q {
            return;
        }
        self.search_panel.q = q.to_string();
        self.restart_search();
    }

    pub fn search_set_scope(&mut self, scope: PanelScope) {
        if self.search_panel.scope == scope {
            return;
        }
        self.search_panel.scope = scope;
        self.restart_search();
    }

    fn restart_search(&mut self) {
        self.abandon_jobs();
        let p = &mut self.search_panel;
        p.run += 1;
        p.more = false;
        p.sug_sel = 0;
        p.resumed = false;
        let run = p.run;
        if p.q.trim().is_empty() {
            p.hits.clear();
            p.next = None;
            p.busy = false;
            self.env.cancel(&Timer::SearchDebounce { run });
        } else {
            p.busy = true;
            self.env.schedule(DEBOUNCE_MS, Timer::SearchDebounce { run });
        }
        self.env.touch();
    }

    fn panel_scope(&self) -> SearchScope {
        let p = &self.search_panel;
        match (&p.origin, p.scope) {
            (Some(o), PanelScope::Buffer) => SearchScope { conn: Some(o.conn.clone()), buf: Some(o.clone()) },
            (Some(o), PanelScope::Network) => SearchScope { conn: Some(o.conn.clone()), buf: None },
            _ => SearchScope::default(),
        }
    }

    /// Ask for the next page of older hits. The UI calls this when the list
    /// scrolls near its end.
    pub fn search_more(&mut self) {
        let p = &self.search_panel;
        let Some(next) = p.next else { return };
        if p.more || p.busy || !p.open {
            return;
        }
        let (q, scope) = (p.q.clone(), self.panel_scope());
        let job = self.search(&q, &scope, Some(next));
        self.search_panel.more = true;
        self.search_panel.more_job = Some(job);
        self.poll_search();
    }

    /// Move the highlighted result; loads more when near the end.
    pub fn search_select(&mut self, sel: usize) {
        let p = &mut self.search_panel;
        p.sel = sel.min(p.hits.len().saturating_sub(1));
        let near_end = p.sel + MORE_AHEAD >= p.hits.len();
        self.env.touch();
        if near_end {
            self.search_more();
        }
    }

    /// Pick up finished pages. Called after any answer arrives.
    pub(crate) fn poll_search(&mut self) {
        let mut changed = false;
        if let Some(job) = self.search_panel.job {
            if let Some(page) = self.take_search(job) {
                let p = &mut self.search_panel;
                p.job = None;
                p.hits = page.hits;
                p.next = page.next;
                p.sel = 0;
                p.busy = false;
                changed = true;
            }
        }
        if let Some(job) = self.search_panel.more_job {
            if let Some(page) = self.take_search(job) {
                let p = &mut self.search_panel;
                p.more_job = None;
                let all = std::mem::take(&mut p.hits).into_iter().chain(page.hits).collect();
                p.hits = dedupe_hits(all);
                p.next = page.next;
                p.more = false;
                changed = true;
            }
        }
        if changed {
            self.env.touch();
        }
    }

    // ── from: and in: suggestions ─────────────────────────────

    /// Names that complete the `from:`/`in:` word at the end of the query.
    pub fn search_suggestions(&self) -> Vec<String> {
        let p = &self.search_panel;
        let Some((kind, part)) = p.token() else { return Vec::new() };
        let part = part.to_lowercase();
        let origin = p.origin.as_ref().and_then(|o| self.conns.get(&o.conn).zip(self.buffer(o)));
        let conns: Vec<&crate::conn::Conn> = match (p.scope, origin) {
            (s, Some((c, _))) if s != PanelScope::All => vec![c],
            _ => self.conns.values().collect(),
        };
        let mut names: Vec<String> = Vec::new();
        let mut add = |n: &str| {
            if !n.is_empty() && !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                names.push(n.to_string());
            }
        };
        if kind == "in" {
            if let Some((_, b)) = origin.filter(|(_, b)| b.kind != crate::buffer::BufferKind::Server) {
                add(&b.name);
            }
            for c in &conns {
                for b in c.all_buffers() {
                    add(&b.name);
                }
            }
        } else {
            // People who spoke here recently first, then everyone else in view.
            if let Some((c, b)) = origin {
                for l in b.lines().iter().rev().take(400) {
                    add(&l.nick);
                }
                for m in b.sorted_members(c.prefix()) {
                    add(&m.nick);
                }
            }
            for c in &conns {
                for b in c.all_buffers().filter(|b| b.kind == crate::buffer::BufferKind::Query) {
                    add(&b.name);
                }
            }
        }
        let all: Vec<String> = names.into_iter().filter(|n| n.to_lowercase().starts_with(&part)).collect();
        if all.len() == 1 && all[0].to_lowercase() == part {
            return Vec::new();
        }
        all.into_iter().take(8).collect()
    }

    /// Finish the `from:`/`in:` word with `name`.
    pub fn search_complete(&mut self, name: &str) {
        let Some((_, part)) = self.search_panel.token() else { return };
        let q = &self.search_panel.q;
        let next = format!("{}{name} ", &q[..q.len() - part.len()]);
        self.search_set_query(&next);
    }

    /// Append a filter (a chip was clicked); the UI moves the caret to the end.
    pub fn search_add_filter(&mut self, prefix: &str) {
        let q = self.search_panel.q.trim();
        let next = if q.is_empty() { prefix.to_string() } else { format!("{q} {prefix}") };
        self.search_set_query(&next);
    }

    /// The input's key handling. True if the key was used.
    pub fn search_key(&mut self, key: SearchKey) -> bool {
        let sugs = self.search_suggestions();
        if !sugs.is_empty() {
            match key {
                SearchKey::Down | SearchKey::Up => {
                    let p = &mut self.search_panel;
                    p.sug_sel = (p.sug_sel + if key == SearchKey::Down { 1 } else { sugs.len() - 1 }) % sugs.len();
                    self.env.touch();
                    return true;
                }
                SearchKey::Enter | SearchKey::Tab => {
                    let pick = sugs[self.search_panel.sug_sel.min(sugs.len() - 1)].clone();
                    self.search_complete(&pick);
                    return true;
                }
            }
        }
        let sel = self.search_panel.sel;
        match key {
            SearchKey::Down => self.search_select(sel + 1),
            SearchKey::Up => self.search_select(sel.saturating_sub(1)),
            SearchKey::Enter => self.search_choose(sel),
            SearchKey::Tab => return false,
        }
        true
    }

    // ── Results ───────────────────────────────────────────────

    /// The buffer name and network a hit belongs to.
    fn hit_where(&self, h: &SearchHit) -> (Option<String>, String) {
        let Some(c) = self.conns.get(&h.network) else { return (None, h.msg.param(0).to_string()) };
        let to = if h.msg.param(0).is_empty() { h.target.as_str() } else { h.msg.param(0) };
        let nick = h.msg.source.as_ref().map(|s| s.nick.as_str()).unwrap_or("");
        let name = if c.is_channel(to) || nick.is_empty() || c.is_me(nick) { to } else { nick };
        let name = if c.fold(name) == h.target { name.to_string() } else { c.buffer(&h.target).map_or_else(|| h.target.clone(), |b| b.name.clone()) };
        (Some(c.name().to_string()), name)
    }

    /// How a hit shows in the list.
    pub fn search_hit_view(&self, h: &SearchHit) -> HitView {
        let (network, buffer) = self.hit_where(h);
        let reachable = network.is_some();
        let raw = h.msg.param(1);
        let text = strip_formatting(&parse_ctcp(raw).map_or(raw.to_string(), |c| c.args));
        let time = hit_time(h);
        HitView {
            buffer,
            network: network.unwrap_or_default(),
            nick: h.msg.source.as_ref().map(|s| s.nick.clone()).unwrap_or_default(),
            parts: marked(&text, &self.search_panel.q, self.now(), self.zone),
            text,
            time,
            stamp: crate::time::format_short(time, self.zone),
            reachable,
        }
    }

    /// Jump to a result and close the panel, remembering the search.
    pub fn search_choose(&mut self, index: usize) {
        let Some(h) = self.search_panel.hits.get(index).cloned() else { return };
        let (net, name) = self.hit_where(&h);
        let Some(_) = net else {
            self.toast("That network is not connected", ToastKind::Error);
            return;
        };
        let Some(c) = self.conns.get_mut(&h.network) else { return };
        let key = c.ensure(&name, None);
        let id = BufId::new(h.network.clone(), key);
        let p = &mut self.search_panel;
        p.last = Some(Snapshot { q: p.q.clone(), origin: p.origin.clone(), scope: p.scope, hits: p.hits.clone(), next: p.next, sel: index });
        self.search_close();
        self.jump_to(&id, hit_time(&h), h.msg.tags.get("msgid").map(String::as_str));
    }
}

/// Split text into plain and matched parts for highlighting the query's words.
pub fn marked(s: &str, q: &str, now: i64, zone: crate::time::Zone) -> Vec<(String, bool)> {
    let words: Vec<String> = parse_query(q, now, zone)
        .text
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '#' || *c == '@').collect::<String>())
        .filter(|w| !w.is_empty())
        .map(|w| regex::escape(&w))
        .collect();
    if words.is_empty() {
        return vec![(s.to_string(), false)];
    }
    let Ok(re) = Regex::new(&format!("(?i)({})", words.join("|"))) else { return vec![(s.to_string(), false)] };
    let (mut out, mut at) = (Vec::new(), 0);
    for m in re.find_iter(s) {
        if m.start() > at {
            out.push((s[at..m.start()].to_string(), false));
        }
        out.push((m.as_str().to_string(), true));
        at = m.end();
    }
    if at < s.len() {
        out.push((s[at..].to_string(), false));
    }
    out
}

/// The debounce fired: start the search, unless the query moved on.
pub(crate) fn debounce_fired(app: &mut App, run: u64) {
    let p = &app.search_panel;
    if !p.open || p.run != run || p.q.trim().is_empty() {
        return;
    }
    let (q, scope) = (p.q.clone(), app.panel_scope());
    let job = app.search(&q, &scope, None);
    app.search_panel.job = Some(job);
    app.poll_search();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Zone;

    #[test]
    fn marks_the_queried_words() {
        let parts = marked("Deploy failed on deploy", "deploy from:ann", 0, Zone::UTC);
        assert_eq!(parts, vec![("Deploy".into(), true), (" failed on ".into(), false), ("deploy".into(), true)]);
        assert_eq!(marked("plain", "from:ann", 0, Zone::UTC), vec![("plain".to_string(), false)]);
    }

    #[test]
    fn spots_the_word_being_typed() {
        let p = SearchPanel { q: "bug from:an".into(), ..Default::default() };
        assert_eq!(p.token(), Some(("from".into(), "an".into())));
        assert_eq!(SearchPanel { q: "bug".into(), ..Default::default() }.token(), None);
    }
}
