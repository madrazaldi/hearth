//! Backfilling buffers: CHATHISTORY when the server has it, else the local log.

use crate::buffer::{BufId, Buffer, BufferKind, Focus, Gap, HistoryState};
use crate::effect::{Effect, HistoryKind, Reply, ReqId};
use crate::incoming::history_line;
use crate::irc::{iso_time, msg_time};
use crate::state::{App, Pending};
use crate::types::{ConnStatus, IrcMessage, Line};
use crate::Conn;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistMode {
    Latest,
    Before,
    After,
}

/// Which edge of a gap a page is loaded from: `After` is its older edge
/// (scrolling down into it), `Before` its newer one (scrolling up).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapFrom {
    After,
    Before,
}

/// Where a jump's "load around" request stands.
pub(crate) enum AroundStage {
    Remote,
    LocalBefore,
    LocalAfter(Vec<IrcMessage>),
}

fn line_key(l: &Line) -> String {
    l.msgid.clone().unwrap_or_else(|| format!("{}|{:?}|{}|{}", l.time, l.kind, l.nick, l.text))
}

/// `existing` plus the lines of `incoming` it doesn't already have, sorted by
/// time. Returns `None` when nothing is new.
pub fn merge_lines(existing: &[Line], incoming: Vec<Line>) -> Option<Vec<Line>> {
    if incoming.is_empty() {
        return None;
    }
    let seen: HashSet<String> = existing.iter().map(line_key).collect();
    // Lines without a msgid can differ by a few ms between the live copy and
    // the logged one (our own messages without echo-message), so also match
    // same-content lines that are close in time.
    let mut loose: HashMap<String, Vec<i64>> = HashMap::new();
    for l in existing.iter().filter(|l| l.msgid.is_none()) {
        loose.entry(format!("{:?}|{}|{}", l.kind, l.nick, l.text)).or_default().push(l.time);
    }
    let fresh: Vec<Line> = incoming
        .into_iter()
        .filter(|l| {
            if seen.contains(&line_key(l)) {
                return false;
            }
            if l.msgid.is_some() {
                return true;
            }
            !loose.get(&format!("{:?}|{}|{}", l.kind, l.nick, l.text)).is_some_and(|ts| ts.iter().any(|t| (t - l.time).abs() < 3000))
        })
        .collect();
    if fresh.is_empty() {
        return None;
    }
    let mut all: Vec<Line> = existing.iter().cloned().chain(fresh).collect();
    all.sort_by(|a, b| a.time.cmp(&b.time).then(a.id.cmp(&b.id)));
    Some(all)
}

/// The CHATHISTORY command to page `buf` in `mode`, and the limit it asks for.
pub fn chathistory_request(buf: &Buffer, mode: HistMode, max: usize) -> (usize, String) {
    let (first, last) = (buf.lines().first(), buf.last_line());
    if let (HistMode::After, Some(last)) = (mode, last) {
        let limit = max.min(1000);
        return (limit, format!("CHATHISTORY AFTER {} timestamp={} {limit}", buf.name, iso_time(last.time)));
    }
    let limit = max.min(100);
    if let (HistMode::Before, Some(first)) = (mode, first) {
        return (limit, format!("CHATHISTORY BEFORE {} timestamp={} {limit}", buf.name, iso_time(first.time)));
    }
    (limit, format!("CHATHISTORY LATEST {} * {limit}", buf.name))
}

fn remote(conn: &Conn) -> bool {
    conn.has("draft/chathistory") && conn.status == ConnStatus::Connected
}

fn chathistory_max(conn: &Conn) -> usize {
    conn.isupport.get("CHATHISTORY").and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(100)
}

pub(crate) fn fetch_history(app: &mut App, conn: &mut Conn, key: &str, mode: HistMode) {
    if key.is_empty() {
        return;
    }
    let remote = remote(conn);
    // The local log only fills gaps before what we have: anything newer
    // arrived live in this session.
    if !remote && (!app.settings.store_logs || mode == HistMode::After) {
        return;
    }
    let max = chathistory_max(conn);
    let conn_id = conn.id.clone();
    let Some(b) = conn.buf_mut(key) else { return };
    if b.history == HistoryState::Loading || (mode == HistMode::Before && b.history == HistoryState::Done) {
        return;
    }
    b.history = HistoryState::Loading;
    let req = app.env.next_req();
    b.pending = Some(req);
    b.touch();
    let limit;
    if remote {
        let (l, line) = chathistory_request(b, mode, max);
        limit = l;
        app.env.emit(Effect::History { req, conn: conn_id, kind: HistoryKind::Messages, target: b.name.clone(), line });
    } else {
        limit = 100;
        let before = if mode == HistMode::Before { b.lines().first().map(|l| l.time) } else { None };
        app.env.emit(Effect::LocalHistory { req, conn: conn_id, target: key.to_string(), before, after: None, limit });
    }
    let id = b.id();
    app.pending.insert(req, Pending::History { buf: id, mode, limit });
}

pub(crate) fn history_done(app: &mut App, req: ReqId, id: BufId, mode: HistMode, limit: usize, result: Result<Reply, String>) {
    app.with_conn(&id.conn, |app, conn| {
        let Some(b) = conn.buf_mut(&id.key) else { return };
        if b.pending == Some(req) {
            b.pending = None;
        }
        match result {
            Ok(Reply::Messages(msgs)) => {
                apply_history(app, conn, &id.key, &msgs);
                let b = conn.buf_mut(&id.key).unwrap();
                b.history = if mode != HistMode::After && msgs.len() < limit { HistoryState::Done } else { HistoryState::Idle };
            }
            _ => b.history = HistoryState::Idle,
        }
        let b = conn.buf_mut(&id.key).unwrap();
        b.touch();
        if let Some((time, msgid)) = b.jump_after_pending.take() {
            continue_jump(app, conn, &id.key, time, msgid);
        }
    });
}

/// The gaps once a contiguous window of history, from `w0` to `w1`, has been
/// merged into `lines`. The window fills whatever gaps it covers; if it touches
/// no loaded line and no gap, it's an island and gets a gap to its neighbour.
pub fn add_window(gaps: &[Gap], lines: &[Line], w0: i64, w1: i64) -> Vec<Gap> {
    let mut out = Vec::new();
    let mut touches = lines.iter().any(|l| l.time >= w0 && l.time <= w1);
    for g in gaps {
        if g.before <= w0 || g.after >= w1 {
            out.push(*g);
            continue;
        }
        touches = true;
        if g.after < w0 {
            out.push(Gap { after: g.after, before: w0 });
        }
        if w1 < g.before {
            out.push(Gap { after: w1, before: g.before });
        }
    }
    if !touches {
        let prev = lines.iter().rev().find(|l| l.time < w0);
        let next = lines.iter().find(|l| l.time > w1);
        if let Some(p) = prev {
            out.push(Gap { after: p.time, before: w0 });
        }
        if let Some(n) = next {
            out.push(Gap { after: w1, before: n.time });
        }
    }
    out.sort_by_key(|g| g.after);
    out
}

/// Merge a contiguous window of history and update the buffer's gaps.
fn apply_window(app: &mut App, conn: &mut Conn, key: &str, msgs: &[IrcMessage], w0: i64, w1: i64) {
    if let Some(b) = conn.buf_mut(key) {
        b.gaps = add_window(&b.gaps, b.lines(), w0, w1);
    }
    apply_history(app, conn, key, msgs);
}

fn apply_history(app: &mut App, conn: &mut Conn, key: &str, msgs: &[IrcMessage]) {
    let lines: Vec<Line> = msgs.iter().filter_map(|m| history_line(app, conn, m)).collect();
    let reactions: Vec<(String, String, String)> = msgs
        .iter()
        .filter_map(|m| {
            let (emoji, parent) = (m.tags.get("+draft/react")?, m.tags.get("+draft/reply")?);
            let src = m.source.as_ref().filter(|s| m.command == "TAGMSG" && !app.is_ignored(conn, Some(s)))?;
            Some((parent.clone(), emoji.clone(), src.nick.clone()))
        })
        .collect();
    let Some(b) = conn.buf_mut(key) else { return };
    b.merge(lines);
    for (parent, emoji, nick) in reactions {
        b.react(&parent, &emoji, &nick);
    }
}

// ── Jumping ───────────────────────────────────────────────────

fn is_present(b: &Buffer, time: i64, msgid: &Option<String>) -> bool {
    b.lines().iter().any(|l| match (msgid, &l.msgid) {
        (Some(want), Some(have)) => want == have,
        _ => l.time == time,
    })
}

fn set_focus(app: &App, conn: &mut Conn, key: &str, time: i64, msgid: Option<String>) {
    let seq = app.env.next_seq();
    if let Some(b) = conn.buf_mut(key) {
        b.focus = Some(Focus { time, msgid, seq });
        b.touch();
    }
}

/// Scroll to a message, loading history around it first if it isn't loaded.
/// The buffer's `focus` is set when that's done.
pub(crate) fn jump_to(app: &mut App, id: &BufId, time: i64, msgid: Option<String>) {
    let key = id.key.clone();
    app.with_conn(&id.conn, |app, conn| {
        let Some(b) = conn.buf_mut(&key) else { return };
        if is_present(b, time, &msgid) {
            return set_focus(app, conn, &key, time, msgid);
        }
        // A buffer that's still loading its newest lines: let them land first,
        // so the jump knows where the gap to the present is.
        if b.pending.is_some() {
            b.jump_after_pending = Some((time, msgid));
            return;
        }
        load_around(app, conn, &key, time, msgid);
    });
}

fn continue_jump(app: &mut App, conn: &mut Conn, key: &str, time: i64, msgid: Option<String>) {
    if conn.buf(key).is_some_and(|b| is_present(b, time, &msgid)) {
        return set_focus(app, conn, key, time, msgid);
    }
    load_around(app, conn, key, time, msgid);
}

/// Load history around a message so it can be scrolled to.
fn load_around(app: &mut App, conn: &mut Conn, key: &str, time: i64, msgid: Option<String>) {
    let Some(b) = conn.buf(key) else { return };
    let (name, id) = (b.name.clone(), b.id());
    let req = app.env.next_req();
    if remote(conn) {
        // soju and others only take timestamps (MSGREFTYPES), and every hit has one.
        let by_id = msgid.as_ref().filter(|_| conn.isupport.get("MSGREFTYPES").is_some_and(|v| v.split(',').any(|t| t == "msgid")));
        let at = match by_id {
            Some(m) => format!("msgid={m}"),
            None => format!("timestamp={}", iso_time(time)),
        };
        app.env.emit(Effect::History {
            req,
            conn: conn.id.clone(),
            kind: HistoryKind::Messages,
            target: name.clone(),
            line: format!("CHATHISTORY AROUND {name} {at} 100"),
        });
        app.pending.insert(req, Pending::Around { buf: id, time, msgid, stage: AroundStage::Remote });
    } else if app.settings.store_logs {
        app.env.emit(Effect::LocalHistory { req, conn: conn.id.clone(), target: key.to_string(), before: Some(time + 1), after: None, limit: 60 });
        app.pending.insert(req, Pending::Around { buf: id, time, msgid, stage: AroundStage::LocalBefore });
    } else {
        set_focus(app, conn, key, time, msgid);
    }
}

pub(crate) fn around_done(app: &mut App, id: BufId, time: i64, msgid: Option<String>, stage: AroundStage, result: Result<Reply, String>) {
    app.with_conn(&id.conn, |app, conn| {
        let msgs = match (result, stage) {
            (Ok(Reply::Messages(m)), AroundStage::Remote) => m,
            (Ok(Reply::Messages(m)), AroundStage::LocalBefore) => {
                // The newer half is a second request.
                let req = app.env.next_req();
                app.env.emit(Effect::LocalHistory { req, conn: conn.id.clone(), target: id.key.clone(), before: None, after: Some(time), limit: 60 });
                app.pending.insert(req, Pending::Around { buf: id.clone(), time, msgid, stage: AroundStage::LocalAfter(m) });
                return;
            }
            (Ok(Reply::Messages(m)), AroundStage::LocalAfter(mut before)) => {
                before.extend(m);
                before
            }
            _ => Vec::new(),
        };
        if let (Some(first), Some(last)) = (msgs.first(), msgs.last()) {
            let now = app.now();
            apply_window(app, conn, &id.key, &msgs, msg_time(&first.tags, now), msg_time(&last.tags, now));
        }
        set_focus(app, conn, &id.key, time, msgid);
    });
}

// ── Gaps ──────────────────────────────────────────────────────

/// Whether [`fetch_gap`] can do anything right now.
pub fn can_fill(app: &App, conn: &Conn) -> bool {
    remote(conn) || app.settings.store_logs
}

/// Load a page of the lines missing in `gap`, from its older edge (`After`,
/// when scrolling down into it) or its newer one (`Before`, scrolling up).
pub(crate) fn fetch_gap(app: &mut App, conn: &mut Conn, key: &str, gap: Gap, from: GapFrom) {
    let now = app.now();
    if !can_fill(app, conn) {
        return;
    }
    let is_remote = remote(conn);
    let max = chathistory_max(conn);
    let conn_id = conn.id.clone();
    let Some(b) = conn.buf_mut(key) else { return };
    if b.filling_gap || now < b.gap_retry_at {
        return;
    }
    b.filling_gap = true;
    let req = app.env.next_req();
    let limit;
    if is_remote {
        limit = max.min(200);
        let (a, z) = if from == GapFrom::After { (gap.after, gap.before) } else { (gap.before, gap.after) };
        let line = format!("CHATHISTORY BETWEEN {} timestamp={} timestamp={} {limit}", b.name, iso_time(a), iso_time(z));
        app.env.emit(Effect::History { req, conn: conn_id, kind: HistoryKind::Messages, target: b.name.clone(), line });
    } else {
        limit = 200;
        let (before, after) = if from == GapFrom::After { (None, Some(gap.after)) } else { (Some(gap.before), None) };
        app.env.emit(Effect::LocalHistory { req, conn: conn_id, target: key.to_string(), before, after, limit });
    }
    let id = b.id();
    app.pending.insert(req, Pending::Gap { buf: id, gap, from, remote: is_remote, limit });
}

pub(crate) fn gap_done(app: &mut App, id: BufId, gap: Gap, from: GapFrom, is_remote: bool, limit: usize, result: Result<Reply, String>) {
    app.with_conn(&id.conn, |app, conn| {
        let now = app.now();
        let Some(b) = conn.buf_mut(&id.key) else { return };
        b.filling_gap = false;
        let Ok(Reply::Messages(raw)) = result else {
            b.gap_retry_at = now + 5000;
            return;
        };
        let (msgs, done): (Vec<IrcMessage>, bool) = if is_remote {
            let done = raw.len() < limit;
            (raw, done)
        } else {
            let total = raw.len();
            let kept: Vec<IrcMessage> = raw
                .into_iter()
                .filter(|m| {
                    let t = msg_time(&m.tags, now);
                    t > gap.after && t < gap.before
                })
                .collect();
            let done = total < limit || kept.len() < total;
            (kept, done)
        };
        let (mut w0, mut w1) = (gap.after, gap.before);
        // A page that doesn't move past the edge (all on one timestamp) would
        // be fetched again forever; treat it as the whole gap.
        if !done {
            if let (Some(first), Some(last)) = (msgs.first(), msgs.last()) {
                match from {
                    GapFrom::After => w1 = msg_time(&last.tags, now).max(gap.after + 1),
                    GapFrom::Before => w0 = msg_time(&first.tags, now).min(gap.before - 1),
                }
            }
        }
        apply_window(app, conn, &id.key, &msgs, w0, w1);
    });
}

/// Open query buffers for recent conversations the server remembers.
pub(crate) fn fetch_targets(app: &mut App, conn: &mut Conn) {
    if !conn.has("draft/chathistory") || conn.is_control() {
        return;
    }
    let now = app.now();
    let line = format!("CHATHISTORY TARGETS timestamp={} timestamp={} 50", iso_time(now + 60_000), iso_time(now - 14 * 86_400_000));
    let req = app.env.next_req();
    app.env.emit(Effect::History { req, conn: conn.id.clone(), kind: HistoryKind::Targets, target: "*".into(), line });
    app.pending.insert(req, Pending::Targets { conn: conn.id.clone() });
}

pub(crate) fn targets_done(app: &mut App, conn_id: &str, result: Result<Reply, String>) {
    let Ok(Reply::Messages(msgs)) = result else { return };
    app.with_conn(conn_id, |app, conn| {
        for m in msgs {
            let target = m.param(1);
            if target.is_empty() || conn.is_channel(target) || conn.is_me(target) {
                continue;
            }
            let key = conn.ensure(target, Some(BufferKind::Query));
            if conn.buf(&key).is_some_and(|b| b.lines().is_empty()) {
                fetch_history(app, conn, &key, HistMode::Latest);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LineType;

    const BASE: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z
    fn t(s: i64) -> i64 {
        BASE + s * 1000
    }
    fn line(id: u64, s: i64) -> Line {
        Line::new(id, t(s), LineType::Privmsg, "ann", format!("t{s}"))
    }

    #[test]
    fn add_window_leaves_a_gap_between_an_old_window_and_the_present() {
        let lines = [line(1, 100), line(2, 101), line(3, 102)];
        assert_eq!(add_window(&[], &lines, t(10), t(20)), vec![Gap { after: t(20), before: t(100) }]);
        assert_eq!(add_window(&[], &lines, t(50), t(101)), vec![]);
    }

    #[test]
    fn add_window_shrinks_or_splits_the_gaps_it_covers() {
        let lines = [line(1, 100), line(2, 101), line(3, 102)];
        let gap = [Gap { after: t(20), before: t(100) }];
        assert_eq!(add_window(&gap, &lines, t(20), t(40)), vec![Gap { after: t(40), before: t(100) }]);
        assert_eq!(add_window(&gap, &lines, t(60), t(100)), vec![Gap { after: t(20), before: t(60) }]);
        assert_eq!(add_window(&gap, &lines, t(40), t(60)), vec![Gap { after: t(20), before: t(40) }, Gap { after: t(60), before: t(100) }]);
        assert_eq!(add_window(&gap, &lines, t(20), t(100)), vec![]);
    }

    #[test]
    fn merge_skips_known_lines_and_near_duplicates() {
        let mut a = line(1, 10);
        a.msgid = Some("m1".into());
        let loose = line(2, 20);
        let existing = vec![a.clone(), loose.clone()];
        assert!(merge_lines(&existing, vec![a.clone()]).is_none());
        // Same content a second apart, no msgid: the logged copy of our own line.
        let mut dup = loose.clone();
        dup.id = 9;
        dup.time += 1000;
        assert!(merge_lines(&existing, vec![dup]).is_none());
        let mut newer = line(3, 5);
        newer.text = "other".into();
        let merged = merge_lines(&existing, vec![newer]).unwrap();
        assert_eq!(merged.iter().map(|l| l.id).collect::<Vec<_>>(), vec![3, 1, 2]);
    }
}
