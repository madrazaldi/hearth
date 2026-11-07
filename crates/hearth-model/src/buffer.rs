//! A buffer: one channel, conversation, or a connection's server log.

use crate::composer::Composer;
use crate::effect::ReqId;
use crate::env::Env;
use crate::history::merge_lines;
use crate::irc::{Fold, PrefixMap};
use crate::types::{Line, LineType, NotifyLevel};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

/// Rows are windowed by the UI, so the cap is about memory and the O(n)
/// derivations per new line (unread counts, msgid index, timeline).
pub const MAX_LINES: usize = 20_000;
/// Trimmed in chunks so a busy channel doesn't shift the array on every line.
pub const TRIM_TO: usize = 18_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BufferKind {
    Server,
    Channel,
    Query,
}

/// Names a buffer within the app: its connection and its folded name
/// (empty for the connection's server buffer). Renaming a buffer changes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BufId {
    pub conn: String,
    pub key: String,
}

impl BufId {
    pub fn new(conn: impl Into<String>, key: impl Into<String>) -> BufId {
        BufId { conn: conn.into(), key: key.into() }
    }

    pub fn server(conn: impl Into<String>) -> BufId {
        BufId { conn: conn.into(), key: String::new() }
    }

    pub fn is_server(&self) -> bool {
        self.key.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Member {
    pub nick: String,
    /// Prefix symbols, highest rank first (`@+`).
    pub prefix: String,
    pub away: bool,
    pub account: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
    pub realname: Option<String>,
}

impl Member {
    pub fn new(nick: impl Into<String>) -> Member {
        Member { nick: nick.into(), ..Member::default() }
    }

    /// The highest-ranked prefix symbol, if any.
    pub fn top_prefix(&self) -> Option<char> {
        self.prefix.chars().next()
    }
}

/// An unloaded stretch between loaded lines: lines newer than `after` and
/// older than `before` may be missing. Both are times of loaded lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    pub after: i64,
    pub before: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryState {
    #[default]
    Idle,
    Loading,
    Done,
}

/// A line to scroll to and flash, set by search results and reply quotes.
/// `seq` changes on every request, so asking for the same line twice works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Focus {
    pub time: i64,
    pub msgid: Option<String>,
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub unread: usize,
    pub highlights: usize,
    /// Highlights counting every message, for buffers notifying on all.
    pub all: usize,
}

/// Locale-ish ordering for names: punctuation, then digits, then letters.
pub(crate) fn collate(a: &str, b: &str) -> Ordering {
    fn key(c: char) -> (u8, char) {
        if c.is_alphabetic() {
            (2, c)
        } else if c.is_numeric() {
            (1, c)
        } else {
            (0, c)
        }
    }
    a.chars().map(key).cmp(b.chars().map(key))
}

pub struct Buffer {
    pub(crate) env: Env,
    pub conn_id: String,
    /// The connection's stable preference key (`serverId|netid`).
    pub(crate) conn_pref: String,
    pub kind: BufferKind,
    pub name: String,
    pub(crate) fold: Fold,
    lines: Vec<Line>,
    pub(crate) members: BTreeMap<String, Member>,
    pub topic: String,
    pub topic_by: String,
    pub topic_time: i64,
    pub modes: String,
    pub joined: bool,
    pub read_marker: i64,
    /// Where the "New" divider goes; 0 for none. Set when the buffer is opened.
    pub divider: i64,
    pub history: HistoryState,
    /// Unloaded stretches between loaded lines, oldest first.
    pub gaps: Vec<Gap>,
    /// Nick → when the typing indicator lapses (ms).
    pub typing: BTreeMap<String, i64>,
    /// The message the composer is replying to.
    pub reply_to: Option<Line>,
    pub focus: Option<Focus>,
    pub want_joined: bool,
    /// Channel key, remembered for rejoining.
    pub join_key: String,
    pub composer: Composer,
    pub(crate) names_acc: Option<BTreeMap<String, Member>>,
    pub(crate) last_typing_sent: i64,
    /// The latest-lines request in flight, if any.
    pub(crate) pending: Option<ReqId>,
    pub(crate) filling_gap: bool,
    pub(crate) gap_retry_at: i64,
    /// A jump waiting for the in-flight history request to land.
    pub(crate) jump_after_pending: Option<(i64, Option<String>)>,
    rev: u64,
    lines_rev: u64,
    counts: Cell<Option<(u64, i64, Counts)>>,
    msgid_index: RefCell<(u64, HashMap<String, usize>)>,
}

impl Buffer {
    pub(crate) fn new(env: &Env, conn_id: &str, conn_pref: &str, fold: Fold, has_read_marker: bool, kind: BufferKind, name: &str) -> Buffer {
        Buffer {
            env: env.clone(),
            conn_id: conn_id.into(),
            conn_pref: conn_pref.into(),
            kind,
            name: name.into(),
            fold,
            lines: Vec::new(),
            members: BTreeMap::new(),
            topic: String::new(),
            topic_by: String::new(),
            topic_time: 0,
            modes: String::new(),
            joined: false,
            // Without server-side markers everything already here counts as read.
            read_marker: if has_read_marker { 0 } else { env.now() },
            divider: 0,
            history: HistoryState::Idle,
            gaps: Vec::new(),
            typing: BTreeMap::new(),
            reply_to: None,
            focus: None,
            want_joined: false,
            join_key: String::new(),
            composer: Composer::default(),
            names_acc: None,
            last_typing_sent: 0,
            pending: None,
            filling_gap: false,
            gap_retry_at: 0,
            jump_after_pending: None,
            rev: 1,
            lines_rev: 1,
            counts: Cell::new(None),
            msgid_index: RefCell::new((0, HashMap::new())),
        }
    }

    /// Folded name; empty for the server buffer.
    pub fn key(&self) -> String {
        if self.kind == BufferKind::Server {
            String::new()
        } else {
            self.fold.apply(&self.name)
        }
    }

    pub fn id(&self) -> BufId {
        BufId { conn: self.conn_id.clone(), key: self.key() }
    }

    /// Stable across restarts; used for saved preferences.
    pub fn pref_key(&self) -> String {
        format!("{}|{}", self.conn_pref, self.key())
    }

    /// Bumps whenever anything about the buffer changes; the UI compares it
    /// to know whether to rebuild this buffer's view.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Bumps when the set of lines (or a line's content) changes, so a
    /// message list can reuse its layout while only members or topic moved.
    pub fn lines_rev(&self) -> u64 {
        self.lines_rev
    }

    pub(crate) fn touch(&mut self) {
        self.rev += 1;
        self.env.touch();
    }

    fn touch_lines(&mut self) {
        self.lines_rev += 1;
        self.touch();
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn last_line(&self) -> Option<&Line> {
        self.lines.last()
    }

    /// Replaces every line (tests, `/clear`).
    pub fn set_lines(&mut self, lines: Vec<Line>) {
        self.lines = lines;
        self.touch_lines();
    }

    pub fn clear(&mut self) {
        self.set_lines(Vec::new());
        self.gaps.clear();
    }

    pub(crate) fn set_fold(&mut self, fold: Fold) {
        self.fold = fold;
    }

    pub fn find_msgid(&self, msgid: &str) -> Option<&Line> {
        let mut idx = self.msgid_index.borrow_mut();
        if idx.0 != self.lines_rev {
            idx.1.clear();
            for (i, l) in self.lines.iter().enumerate() {
                if let Some(m) = &l.msgid {
                    idx.1.insert(m.clone(), i);
                }
            }
            idx.0 = self.lines_rev;
        }
        idx.1.get(msgid).and_then(|i| self.lines.get(*i))
    }

    pub fn line_by_id(&self, id: u64) -> Option<&Line> {
        self.lines.iter().rev().find(|l| l.id == id)
    }

    /// Unread messages from others after the read marker.
    pub fn unread(&self) -> usize {
        self.counts().unread
    }

    /// Mentions waiting (every message, in a query or when notifying on all).
    pub fn highlights(&self, level: Option<NotifyLevel>) -> usize {
        if level == Some(NotifyLevel::None) {
            return 0;
        }
        let c = self.counts();
        if self.kind == BufferKind::Query || level == Some(NotifyLevel::All) {
            c.all
        } else {
            c.highlights
        }
    }

    fn counts(&self) -> Counts {
        if let Some((rev, marker, c)) = self.counts.get() {
            if rev == self.lines_rev && marker == self.read_marker {
                return c;
            }
        }
        let c = Counts {
            unread: crate::unread::count_unread(&self.lines, self.read_marker),
            highlights: crate::unread::count_highlights(&self.lines, self.read_marker, false),
            all: crate::unread::count_highlights(&self.lines, self.read_marker, true),
        };
        self.counts.set(Some((self.lines_rev, self.read_marker, c)));
        c
    }

    pub fn set_read_marker(&mut self, t: i64) {
        if self.read_marker != t {
            self.read_marker = t;
            self.touch();
        }
    }

    pub fn push(&mut self, mut line: Line) {
        self.stamp_prefix(&mut line);
        // Live lines are almost always newest; keep order if a server-time tag says otherwise.
        let at = match self.lines.last() {
            Some(last) if last.time > line.time => self.lines.partition_point(|l| l.time <= line.time),
            _ => self.lines.len(),
        };
        self.lines.insert(at, line);
        if self.lines.len() > MAX_LINES {
            self.lines.drain(..self.lines.len() - TRIM_TO);
            self.history = HistoryState::Idle;
            // Paging back from the new oldest line covers a gap that was cut off.
            if let Some(first) = self.lines.first().map(|l| l.time) {
                self.gaps.retain(|g| g.after >= first);
            }
        }
        self.touch_lines();
    }

    /// Add lines from history, skipping ones already present.
    pub fn merge(&mut self, mut incoming: Vec<Line>) {
        for l in &mut incoming {
            self.stamp_prefix(l);
        }
        if let Some(next) = merge_lines(&self.lines, incoming) {
            self.lines = next;
            self.touch_lines();
        }
    }

    /// Record a +draft/react on the message with `msgid`.
    pub fn react(&mut self, msgid: &str, emoji: &str, nick: &str) {
        let fold = self.fold;
        let Some(l) = self.lines.iter_mut().find(|l| l.msgid.as_deref() == Some(msgid)) else { return };
        match l.reactions.iter_mut().find(|(e, _)| e == emoji) {
            Some((_, who)) => {
                if who.iter().any(|n| fold.apply(n) == fold.apply(nick)) {
                    return;
                }
                who.push(nick.to_string());
            }
            None => l.reactions.push((emoji.to_string(), vec![nick.to_string()])),
        }
        self.touch_lines();
    }

    /// Mark a message deleted (draft/message-redaction).
    pub fn redact(&mut self, msgid: &str) {
        let mut changed = false;
        for l in self.lines.iter_mut().filter(|l| l.msgid.as_deref() == Some(msgid)) {
            l.redacted = true;
            l.reactions.clear();
            changed = true;
        }
        if changed {
            self.touch_lines();
        }
    }

    /// Stamped on the line rather than looked up when rendering, so rows don't
    /// change on every join and mode change. History gets the current prefix.
    fn stamp_prefix(&self, l: &mut Line) {
        if self.kind != BufferKind::Channel || l.prefix.is_some() || l.nick.is_empty() {
            return;
        }
        if !matches!(l.kind, LineType::Privmsg | LineType::Action | LineType::Notice) {
            return;
        }
        if let Some(p) = self.member(&l.nick).and_then(|m| m.top_prefix()) {
            l.prefix = Some(p.to_string());
        }
    }

    pub fn member(&self, nick: &str) -> Option<&Member> {
        self.members.get(&self.fold.apply(nick))
    }

    pub fn set_member(&mut self, m: Member) {
        self.members.insert(self.fold.apply(&m.nick), m);
        self.touch();
    }

    pub fn remove_member(&mut self, nick: &str) -> bool {
        let had = self.members.remove(&self.fold.apply(nick)).is_some();
        if had {
            self.touch();
        }
        had
    }

    pub fn clear_members(&mut self) {
        if !self.members.is_empty() {
            self.members.clear();
            self.touch();
        }
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Members unordered.
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.members.values()
    }

    /// Members by rank, then name.
    pub fn sorted_members(&self, prefix: &PrefixMap) -> Vec<&Member> {
        let rank = |m: &Member| m.top_prefix().and_then(|c| prefix.rank(c)).unwrap_or(prefix.symbols.chars().count());
        let mut v: Vec<&Member> = self.members.values().collect();
        v.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| collate(&self.fold.apply(&a.nick), &self.fold.apply(&b.nick))));
        v
    }
}
