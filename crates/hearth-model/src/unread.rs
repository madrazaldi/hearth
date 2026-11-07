//! Unread and highlight counts, and the read marker (draft/read-marker).

use crate::env::Timer;
use crate::types::{Line, LineType};
use crate::Conn;

fn counted(t: LineType) -> bool {
    matches!(t, LineType::Privmsg | LineType::Action | LineType::Notice)
}

/// Messages from others after `read_marker`.
pub fn count_unread(lines: &[Line], read_marker: i64) -> usize {
    lines.iter().rev().take_while(|l| l.time > read_marker).filter(|l| !l.is_self && counted(l.kind)).count()
}

/// Highlights after `read_marker`; with `everything`, every message counts.
pub fn count_highlights(lines: &[Line], read_marker: i64, everything: bool) -> usize {
    lines.iter().rev().take_while(|l| l.time > read_marker).filter(|l| !l.is_self && (l.highlight || (everything && counted(l.kind)))).count()
}

/// Mark everything in the buffer read and, with draft/read-marker, tell the server (debounced).
pub(crate) fn mark_read(conn: &mut Conn, key: &str) {
    let has_marker = conn.has("draft/read-marker");
    let Some(b) = conn.buf_mut(key) else { return };
    let Some(last) = b.last_line().map(|l| l.time) else { return };
    if last <= b.read_marker {
        return;
    }
    b.set_read_marker(last);
    if key.is_empty() || !has_marker {
        return;
    }
    // Debounced so scrolling through a burst sends one MARKREAD.
    conn.env.schedule(400, Timer::MarkRead { conn: conn.id.clone(), key: key.to_string() });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(time: i64, kind: LineType, is_self: bool, highlight: bool) -> Line {
        let mut l = Line::new(time as u64, time, kind, "x", "t");
        l.is_self = is_self;
        l.highlight = highlight;
        l
    }

    #[test]
    fn counts_only_talk_from_others_after_the_marker() {
        let lines = vec![
            line(1, LineType::Privmsg, false, false),
            line(2, LineType::Join, false, false),
            line(3, LineType::Privmsg, true, false),
            line(4, LineType::Action, false, true),
            line(5, LineType::Notice, false, false),
        ];
        assert_eq!(count_unread(&lines, 0), 3);
        assert_eq!(count_unread(&lines, 4), 1);
        assert_eq!(count_highlights(&lines, 0, false), 1);
        assert_eq!(count_highlights(&lines, 0, true), 3);
    }
}
