//! Accessible names for rows, list navigation math and what to announce.
//! (Focus management itself is the toolkit's job.)

use crate::buffer::{BufferKind, Member};
use crate::format::strip_formatting;
use crate::irc::PrefixMap;
use crate::types::{Line, LineType};

/// A key that moves through a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Down,
    Up,
    Home,
    End,
    PageDown,
    PageUp,
}

/// The index a navigation key moves to from `i` (negative for none) in a
/// list of `count`. `page` is how many rows PageUp/Down skip.
pub fn step_index(key: NavKey, i: i64, count: usize, page: i64) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count as i64 - 1;
    let clamp = |n: i64| n.clamp(0, last) as usize;
    Some(match key {
        NavKey::Down => clamp(i + 1),
        NavKey::Up => clamp(if i < 0 { last } else { i - 1 }),
        NavKey::Home => 0,
        NavKey::End => last as usize,
        NavKey::PageDown => clamp(i + page),
        NavKey::PageUp => clamp(i - page),
    })
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// What a sidebar entry's name is built from.
#[derive(Debug, Clone, Copy)]
pub struct BufferLabel<'a> {
    pub name: &'a str,
    pub kind: BufferKind,
    pub unread: usize,
    pub highlights: usize,
    pub muted: bool,
    pub joined: Option<bool>,
    pub online: Option<bool>,
    pub typing: bool,
}

/// Accessible name of a sidebar entry: its name and what's waiting in it.
pub fn buffer_label(b: &BufferLabel) -> String {
    let query = b.kind == BufferKind::Query;
    let mut parts = vec![b.name.to_string()];
    if let (true, Some(on)) = (query, b.online) {
        parts.push(if on { "online" } else { "offline" }.into());
    }
    if b.kind == BufferKind::Channel && b.joined == Some(false) {
        parts.push("not joined".into());
    }
    if b.highlights > 0 {
        parts.push(plural(b.highlights, if query { "new message" } else { "mention" }));
    }
    if b.unread > 0 && !(query && b.highlights > 0) {
        parts.push(format!("{} unread", b.unread));
    }
    if b.muted {
        parts.push("muted".into());
    }
    if b.typing {
        parts.push("typing".into());
    }
    parts.join(", ")
}

fn rank_name(mode: char) -> String {
    match mode {
        'q' => "owner".into(),
        'a' => "admin".into(),
        'o' => "operator".into(),
        'h' => "half-operator".into(),
        'v' => "voiced".into(),
        m => format!("mode {m}"),
    }
}

/// Accessible name of a member row: nick, rank and away status.
pub fn member_label(m: &Member, prefix: &PrefixMap, display_name: Option<&str>) -> String {
    let mut parts = vec![m.nick.clone()];
    if let Some(d) = display_name.filter(|d| *d != m.nick) {
        parts.push(d.to_string());
    }
    if let Some(sym) = m.top_prefix() {
        parts.push(rank_name(prefix.mode_of(sym).unwrap_or(sym)));
    }
    if m.away {
        parts.push("away".into());
    }
    parts.join(", ")
}

/// Accessible name of a message row: who, when, and what they said.
pub fn line_label(l: &Line, clock: &str, reply_nick: Option<&str>) -> String {
    let text = if l.redacted { "message deleted".to_string() } else { strip_formatting(&l.text) };
    let mut who = l.nick.clone();
    if l.kind == LineType::Notice {
        who += " (notice)";
    }
    let mut s = if l.kind == LineType::Action { format!("{clock}, * {} {text}", l.nick) } else { format!("{who}, {clock}: {text}") };
    if let Some(r) = reply_nick {
        s += &format!(", replying to {r}");
    }
    if !l.redacted && !l.reactions.is_empty() {
        let r: Vec<String> = l.reactions.iter().map(|(e, n)| format!("{e} {}", n.len())).collect();
        s += &format!(". Reactions: {}", r.join(", "));
    }
    s
}

fn is_talk(l: &Line) -> bool {
    matches!(l.kind, LineType::Privmsg | LineType::Notice | LineType::Action)
}

/// What to announce for lines appended after `prev_last`: live messages from
/// others, never history. Short phrases, or a count for a burst.
pub fn announcements(lines: &[Line], prev_last: Option<&Line>, now: i64, max: usize) -> Vec<String> {
    let mut from = lines.len();
    if let Some(p) = prev_last {
        // Only what follows the old last line is new; anything else is a
        // history load or a trim.
        match lines.iter().rposition(|l| l.id == p.id) {
            Some(i) => from = i + 1,
            None => {
                while from > 0 && lines[from - 1].time > p.time {
                    from -= 1;
                }
            }
        }
    }
    // A minute's slack for server clocks; older lines are catch-up history.
    let fresh: Vec<&Line> = lines[from..].iter().filter(|l| is_talk(l) && !l.is_self && now - l.time < 60_000).collect();
    if fresh.len() > max {
        return vec![format!("{} new messages", fresh.len())];
    }
    fresh
        .into_iter()
        .map(|l| {
            let text: String = strip_formatting(&l.text).chars().take(300).collect();
            if l.kind == LineType::Action {
                format!("* {} {text}", l.nick)
            } else {
                format!("{}: {text}", l.nick)
            }
        })
        .collect()
}

pub fn emoji_name(e: &str) -> &str {
    match e {
        "\u{1f44d}" => "thumbs up",
        "\u{2764}\u{fe0f}" => "heart",
        "\u{1f602}" => "laughing",
        "\u{1f389}" => "party",
        "\u{1f62e}" => "surprised",
        "\u{1f622}" => "sad",
        "\u{1f440}" => "eyes",
        "\u{2705}" => "check mark",
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    fn line(id: u64, nick: &str, text: &str) -> Line {
        Line::new(id, NOW - 1000, LineType::Privmsg, nick, text)
    }

    #[test]
    fn steps_and_clamps() {
        assert_eq!(step_index(NavKey::Down, 0, 3, 10), Some(1));
        assert_eq!(step_index(NavKey::Down, 2, 3, 10), Some(2));
        assert_eq!(step_index(NavKey::Up, 0, 3, 10), Some(0));
        assert_eq!(step_index(NavKey::Home, 2, 3, 10), Some(0));
        assert_eq!(step_index(NavKey::End, 0, 3, 10), Some(2));
        assert_eq!(step_index(NavKey::PageDown, 1, 30, 10), Some(11));
        assert_eq!(step_index(NavKey::PageUp, 5, 30, 10), Some(0));
        assert_eq!(step_index(NavKey::Up, -1, 5, 10), Some(4));
        assert_eq!(step_index(NavKey::Down, -1, 5, 10), Some(0));
        assert_eq!(step_index(NavKey::Down, 0, 0, 10), None);
    }

    #[test]
    fn names_buffers_with_their_counts() {
        let b = |name, kind, unread, highlights, muted, joined, online, typing| {
            buffer_label(&BufferLabel { name, kind, unread, highlights, muted, joined, online, typing })
        };
        assert_eq!(b("#hearth", BufferKind::Channel, 4, 1, false, Some(true), None, false), "#hearth, 1 mention, 4 unread");
        assert_eq!(b("#old", BufferKind::Channel, 0, 0, true, Some(false), None, false), "#old, not joined, muted");
        assert_eq!(b("bob", BufferKind::Query, 2, 2, false, None, Some(true), true), "bob, online, 2 new messages, typing");
    }

    #[test]
    fn names_members_with_rank_and_away() {
        let prefix = PrefixMap { modes: "ov".into(), symbols: "@+".into() };
        let m = |nick: &str, p: &str, away| Member { nick: nick.into(), prefix: p.into(), away, ..Member::default() };
        assert_eq!(member_label(&m("alice", "@+", false), &prefix, None), "alice, operator");
        assert_eq!(member_label(&m("bob", "+", true), &prefix, Some("Bob B")), "bob, Bob B, voiced, away");
        assert_eq!(member_label(&m("carol", "", false), &prefix, None), "carol");
    }

    #[test]
    fn names_message_rows() {
        assert_eq!(line_label(&line(1, "alice", "\x02hi\x02 there"), "12:03", None), "alice, 12:03: hi there");
        let mut act = line(2, "alice", "waves");
        act.kind = LineType::Action;
        assert_eq!(line_label(&act, "12:03", None), "12:03, * alice waves");
        let mut n = line(3, "bot", "x");
        n.kind = LineType::Notice;
        n.redacted = true;
        assert_eq!(line_label(&n, "09:00", None), "bot (notice), 09:00: message deleted");
        let mut r = line(4, "a", "yes");
        r.reactions = vec![("\u{1f44d}".into(), vec!["b".into(), "c".into()])];
        assert_eq!(line_label(&r, "1:00p", Some("bob")), "a, 1:00p: yes, replying to bob. Reactions: \u{1f44d} 2");
    }

    #[test]
    fn announces_only_lines_after_the_old_last_one() {
        let (a, b) = (line(1, "a", "one"), line(2, "b", "two"));
        let mut old = line(3, "c", "history");
        old.time = NOW - 3_600_000;
        assert_eq!(announcements(&[old.clone(), a.clone(), b.clone()], Some(&a), NOW, 3), ["b: two"]);
        assert!(announcements(&[old, a, b.clone()], Some(&b), NOW, 3).is_empty());
    }

    #[test]
    fn skips_own_lines_events_and_catch_up() {
        let a = line(1, "a", "one");
        let mut mine = line(2, "me", "mine");
        mine.is_self = true;
        let mut join = line(3, "x", "");
        join.kind = LineType::Join;
        let mut late = line(4, "b", "late");
        late.time = NOW - 3_600_000;
        assert!(announcements(&[a.clone(), mine, join, late], Some(&a), NOW, 3).is_empty());
    }

    #[test]
    fn summarises_a_burst_and_formats_actions() {
        let a = line(1, "a", "one");
        let mut lines = vec![a.clone()];
        lines.extend((2..6).map(|i| line(i, &format!("n{i}"), "m")));
        assert_eq!(announcements(&lines, Some(&a), NOW, 3), ["4 new messages"]);
        let mut w = line(9, "b", "waves");
        w.kind = LineType::Action;
        assert_eq!(announcements(&[a.clone(), w], Some(&a), NOW, 3), ["* b waves"]);
    }

    #[test]
    fn falls_back_to_time_when_the_old_last_line_was_trimmed() {
        let mut gone = line(1, "a", "gone");
        gone.time = NOW - 5000;
        assert_eq!(announcements(&[line(2, "b", "new")], Some(&gone), NOW, 3), ["b: new"]);
    }
}
