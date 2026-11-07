//! A buffer's lines as rows: day and unread dividers, gaps, grouped runs of
//! joins and parts. Items refer to lines by index into `Buffer::lines()`.

use crate::buffer::{BufId, Buffer, BufferKind, Focus, Gap};
use crate::format::strip_formatting;
use crate::state::App;
use crate::time::{format_day, Zone};
use crate::types::{Line, LineType, Membership};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Day {
        key: String,
        time: i64,
        label: String,
    },
    /// The "New" divider.
    Unread {
        key: String,
        time: i64,
    },
    Line {
        key: String,
        time: i64,
        line: usize,
        cont: bool,
    },
    /// Collapsed joins, parts, quits and nick changes.
    Run {
        key: String,
        time: i64,
        lines: Vec<usize>,
    },
    Gap {
        key: String,
        time: i64,
        gap: Gap,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Day,
    Unread,
    Line,
    Run,
    Gap,
}

impl Item {
    pub fn key(&self) -> &str {
        match self {
            Item::Day { key, .. } | Item::Unread { key, .. } | Item::Line { key, .. } | Item::Run { key, .. } | Item::Gap { key, .. } => key,
        }
    }

    pub fn time(&self) -> i64 {
        match self {
            Item::Day { time, .. } | Item::Unread { time, .. } | Item::Line { time, .. } | Item::Run { time, .. } | Item::Gap { time, .. } => *time,
        }
    }

    pub fn kind(&self) -> ItemKind {
        match self {
            Item::Day { .. } => ItemKind::Day,
            Item::Unread { .. } => ItemKind::Unread,
            Item::Line { .. } => ItemKind::Line,
            Item::Run { .. } => ItemKind::Run,
            Item::Gap { .. } => ItemKind::Gap,
        }
    }
}

/// Join/part/quit/nick: the lines the `membership` setting collapses.
pub fn is_membership(t: LineType) -> bool {
    t.is_membership()
}

fn is_talk(t: LineType) -> bool {
    matches!(t, LineType::Privmsg | LineType::Action | LineType::Notice)
}

pub struct TimelineOpts<'a> {
    /// Lines newer than this (and not ours) get a "New" divider before them; 0 for none.
    pub divider: i64,
    /// Membership changes are only grouped in channels.
    pub channel: bool,
    pub membership: Membership,
    /// Unloaded stretches, oldest first; each gets a row before the first line past it.
    pub gaps: &'a [Gap],
    pub zone: Zone,
    pub now: i64,
}

/// Turn lines into rows. `keep` picks which lines take part (a text filter, hidden membership lines).
pub fn build_items(lines: &[Line], keep: impl Fn(&Line) -> bool, o: &TimelineOpts) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut prev: Option<&Line> = None;
    let mut run: Vec<usize> = Vec::new();
    let mut divider_placed = o.divider == 0;
    // Local-day bounds of the current day, so we only touch the calendar at day changes.
    let (mut day_start, mut day_end) = (i64::MAX, i64::MIN);
    let mut g = 0;

    fn flush(run: &mut Vec<usize>, lines: &[Line], membership: Membership, out: &mut Vec<Item>) {
        if run.is_empty() {
            return;
        }
        if run.len() == 1 || membership == Membership::Show {
            for &i in run.iter() {
                out.push(Item::Line { key: format!("l{}", lines[i].id), time: lines[i].time, line: i, cont: false });
            }
        } else {
            out.push(Item::Run { key: format!("r{}", lines[run[0]].id), time: lines[run[0]].time, lines: run.clone() });
        }
        run.clear();
    }

    for (i, l) in lines.iter().enumerate().filter(|(_, l)| keep(l)) {
        while g < o.gaps.len() && o.gaps[g].before <= l.time {
            flush(&mut run, lines, o.membership, &mut out);
            out.push(Item::Gap { key: format!("g{}", o.gaps[g].after), time: o.gaps[g].after, gap: o.gaps[g] });
            g += 1;
            prev = None;
            // Repeat the day after a gap, which can span days.
            day_start = i64::MAX;
        }
        if l.time < day_start || l.time >= day_end {
            (day_start, day_end) = o.zone.day_bounds(l.time);
            flush(&mut run, lines, o.membership, &mut out);
            out.push(Item::Day { key: format!("d{}", l.id), time: l.time, label: format_day(l.time, o.now, o.zone) });
            prev = None;
        }
        if !divider_placed && l.time > o.divider && !l.is_self {
            flush(&mut run, lines, o.membership, &mut out);
            out.push(Item::Unread { key: "u".into(), time: l.time });
            divider_placed = true;
            prev = None;
        }
        if o.channel && is_membership(l.kind) {
            run.push(i);
            prev = Some(l);
            continue;
        }
        flush(&mut run, lines, o.membership, &mut out);
        // A reply shows its quote under a full header, so it starts a new group.
        let cont = prev.is_some_and(|p| {
            l.reply_to.is_none() && is_talk(l.kind) && p.kind == l.kind && l.kind != LineType::Action && p.nick == l.nick && l.time - p.time < 5 * 60_000
        });
        out.push(Item::Line { key: format!("l{}", l.id), time: l.time, line: i, cont });
        prev = Some(l);
    }
    flush(&mut run, lines, o.membership, &mut out);
    out
}

/// Index of the item that shows line `line`, including collapsed runs.
pub fn item_index_of(items: &[Item], line: usize) -> Option<usize> {
    items.iter().position(|it| match it {
        Item::Line { line: l, .. } => *l == line,
        Item::Run { lines, .. } => lines.contains(&line),
        _ => false,
    })
}

/// A buffer's rows, as the message list shows them.
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    pub items: Vec<Item>,
    /// Indices (into `Buffer::lines()`) of the lines that passed the filter.
    pub kept: Vec<usize>,
}

/// Where a [`Focus`] request lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusTarget {
    pub item: usize,
    /// Index into `Buffer::lines()`.
    pub line: usize,
    /// The requested line itself, as opposed to the nearest one after it (flash only the exact one).
    pub exact: bool,
}

impl Timeline {
    pub fn index_of_key(&self, key: &str) -> Option<usize> {
        self.items.iter().position(|it| it.key() == key)
    }

    /// The item to land on for a scroll-to-line request. The exact line can
    /// be missing (deleted, or past the server's history); the nearest one
    /// after it stands in, without the flash.
    pub fn resolve_focus(&self, lines: &[Line], f: &Focus) -> Option<FocusTarget> {
        let exact = self.kept.iter().copied().find(|&i| match (&f.msgid, &lines[i].msgid) {
            (Some(want), Some(have)) => want == have,
            _ => lines[i].time == f.time,
        });
        let line = exact.or_else(|| self.kept.iter().copied().find(|&i| lines[i].time >= f.time)).or_else(|| self.kept.last().copied())?;
        Some(FocusTarget { item: item_index_of(&self.items, line)?, line, exact: exact.is_some() })
    }
}

/// What the message list is built from, to know when a cached timeline is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimelineKey {
    lines_rev: u64,
    divider: i64,
    membership: Membership,
    gaps: Vec<Gap>,
    filter: String,
    zone: Zone,
    day: (i32, u32, u32),
}

impl App {
    /// The rows for a buffer, optionally narrowed by a text filter. Cached
    /// until the buffer's lines, gaps, divider or the display settings change.
    pub fn timeline(&self, id: &BufId, filter: &str) -> Option<Arc<Timeline>> {
        let b = self.buffer(id)?;
        let now = self.now();
        let key = TimelineKey {
            lines_rev: b.lines_rev(),
            divider: b.divider,
            membership: self.settings.membership,
            gaps: b.gaps.clone(),
            filter: filter.trim().to_lowercase(),
            zone: self.zone,
            day: crate::time::day_key(now, self.zone),
        };
        if let Some((k, t)) = self.timeline_cache.borrow().get(id) {
            if *k == key {
                return Some(t.clone());
            }
        }
        let t = Arc::new(build_timeline(b, &key, &self.settings.membership, now, self.zone));
        self.timeline_cache.borrow_mut().insert(id.clone(), (key, t.clone()));
        Some(t)
    }

    /// Take a pending scroll-to-line request for the buffer (consuming it),
    /// resolving it against the rows. The exact line flashes for a moment.
    pub fn take_focus(&mut self, id: &BufId, filter: &str) -> Option<FocusTarget> {
        let f = self.buffer(id)?.focus.clone()?;
        let tl = self.timeline(id, filter)?;
        let b = self.buffer_mut(id)?;
        b.focus = None;
        b.touch();
        let target = tl.resolve_focus(b.lines(), &f)?;
        if target.exact {
            let line_id = b.lines()[target.line].id;
            self.flash = Some(line_id);
            self.env.schedule(1800, crate::env::Timer::ClearFlash { line: line_id });
            self.env.touch();
        }
        Some(target)
    }

    /// The id of the line that's flashing after a jump, if any.
    pub fn flash_line(&self) -> Option<u64> {
        self.flash
    }
}

fn build_timeline(b: &Buffer, key: &TimelineKey, membership: &Membership, now: i64, zone: Zone) -> Timeline {
    let hide = *membership == Membership::Hide && b.kind == BufferKind::Channel;
    let q = key.filter.as_str();
    let keep = |l: &Line| {
        if hide && is_membership(l.kind) {
            return false;
        }
        q.is_empty() || format!("{} {}", l.nick, strip_formatting(&l.text)).to_lowercase().contains(q)
    };
    let opts = TimelineOpts { divider: b.divider, channel: b.kind == BufferKind::Channel, membership: *membership, gaps: &b.gaps, zone, now };
    let kept = b.lines().iter().enumerate().filter(|(_, l)| keep(l)).map(|(i, _)| i).collect();
    Timeline { items: build_items(b.lines(), keep, &opts), kept }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: i64 = 1_767_225_600_000;
    fn t(s: i64) -> i64 {
        BASE + s * 1000
    }
    fn line(id: u64, s: i64, kind: LineType, nick: &str) -> Line {
        Line::new(id, t(s), kind, nick, "x")
    }
    fn opts(gaps: &[Gap]) -> TimelineOpts<'_> {
        TimelineOpts { divider: 0, channel: true, membership: Membership::Collapse, gaps, zone: Zone::UTC, now: t(1000) }
    }

    #[test]
    fn puts_a_gap_row_before_the_first_line_past_it_then_repeats_the_day() {
        let lines = [line(1, 10, LineType::Privmsg, "a"), line(2, 20, LineType::Privmsg, "a"), line(3, 100, LineType::Privmsg, "a")];
        let gaps = [Gap { after: t(20), before: t(100) }];
        let items = build_items(&lines, |_| true, &opts(&gaps));
        let kinds: Vec<_> = items.iter().map(Item::kind).collect();
        assert_eq!(kinds, [ItemKind::Day, ItemKind::Line, ItemKind::Line, ItemKind::Gap, ItemKind::Day, ItemKind::Line]);
    }

    #[test]
    fn groups_talk_and_collapses_membership_runs() {
        let lines = [
            line(1, 1, LineType::Privmsg, "a"),
            line(2, 2, LineType::Privmsg, "a"),
            line(3, 3, LineType::Join, "b"),
            line(4, 4, LineType::Part, "c"),
            line(5, 5, LineType::Privmsg, "a"),
        ];
        let items = build_items(&lines, |_| true, &opts(&[]));
        let conts: Vec<_> = items.iter().filter_map(|i| if let Item::Line { cont, .. } = i { Some(*cont) } else { None }).collect();
        assert_eq!(conts, [false, true, false]);
        assert!(items.iter().any(|i| matches!(i, Item::Run { lines, .. } if lines.len() == 2)));
        // A lone membership line stays a line; `show` never collapses.
        let shown = build_items(&lines, |_| true, &TimelineOpts { membership: Membership::Show, ..opts(&[]) });
        assert!(!shown.iter().any(|i| matches!(i, Item::Run { .. })));
    }

    #[test]
    fn places_the_unread_divider_before_the_first_unseen_line_from_others() {
        let mut mine = line(2, 5, LineType::Privmsg, "me");
        mine.is_self = true;
        let lines = [line(1, 1, LineType::Privmsg, "a"), mine, line(3, 9, LineType::Privmsg, "a")];
        let items = build_items(&lines, |_| true, &TimelineOpts { divider: t(3), ..opts(&[]) });
        let at = items.iter().position(|i| i.kind() == ItemKind::Unread).unwrap();
        assert!(matches!(items[at + 1], Item::Line { line: 2, .. }));
    }
}
