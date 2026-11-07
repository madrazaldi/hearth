//! MONITOR presence: watched nicks (saved per network) plus open queries.

use crate::buffer::BufferKind;
use crate::state::{App, ToastKind};
use crate::Conn;
use std::collections::BTreeMap;

/// MONITOR + commands for `nicks`, keeping each line's target list within 400 bytes.
pub fn monitor_commands(nicks: &[String], limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let mut bytes = 0;
    let flush = |chunk: &mut Vec<&str>, bytes: &mut usize, out: &mut Vec<String>| {
        if !chunk.is_empty() {
            out.push(format!("MONITOR + {}", chunk.join(",")));
        }
        chunk.clear();
        *bytes = 0;
    };
    for n in nicks.iter().take(limit) {
        if !chunk.is_empty() && bytes + 1 + n.len() > 400 {
            flush(&mut chunk, &mut bytes, &mut out);
        }
        bytes += if chunk.is_empty() { 0 } else { 1 } + n.len();
        chunk.push(n);
    }
    flush(&mut chunk, &mut bytes, &mut out);
    out
}

/// Nicks explicitly watched with MONITOR on this connection.
pub fn watched<'a>(app: &'a App, conn: &Conn) -> &'a [String] {
    app.settings.watch.get(&conn.pref_key()).map(Vec::as_slice).unwrap_or(&[])
}

pub fn is_watched(app: &App, conn: &Conn, nick: &str) -> bool {
    let f = conn.fold(nick);
    watched(app, conn).iter().any(|n| conn.fold(n) == f)
}

/// How many nicks the server lets us MONITOR.
fn limit(conn: &Conn) -> usize {
    conn.isupport.get("MONITOR").and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 0).unwrap_or(usize::MAX)
}

/// Everything we MONITOR: watched nicks and open conversations, by folded nick.
fn monitored(app: &App, conn: &Conn) -> BTreeMap<String, String> {
    let mut nicks = BTreeMap::new();
    for n in watched(app, conn) {
        nicks.insert(conn.fold(n), n.clone());
    }
    for b in conn.buffers_iter().filter(|b| b.kind == BufferKind::Query) {
        nicks.insert(conn.fold(&b.name), b.name.clone());
    }
    nicks
}

pub(crate) fn watch(app: &mut App, conn: &mut Conn, nick: &str) {
    if is_watched(app, conn, nick) {
        return;
    }
    if conn.can_monitor() {
        let subs = monitored(app, conn);
        let max = limit(conn);
        if !subs.contains_key(&conn.fold(nick)) && subs.len() >= max {
            return app.toast(format!("Can't watch {nick}: {} allows {max} nicknames", conn.name()), ToastKind::Error);
        }
    }
    let mut list = watched(app, conn).to_vec();
    list.push(nick.to_string());
    let key = conn.pref_key();
    app.update_settings(|s| {
        s.watch.insert(key, list);
    });
    if conn.can_monitor() {
        conn.send(format!("MONITOR + {nick}"));
    }
}

pub(crate) fn unwatch(app: &mut App, conn: &mut Conn, nick: &str) {
    let f = conn.fold(nick);
    let rest: Vec<String> = watched(app, conn).iter().filter(|n| conn.fold(n) != f).cloned().collect();
    let key = conn.pref_key();
    app.update_settings(|s| {
        s.watch.insert(key, rest);
    });
    if conn.can_monitor() && conn.buffer(nick).is_none() {
        conn.send(format!("MONITOR - {nick}"));
        conn.online.remove(&f);
    }
}

/// Subscribe to presence for watched nicks and open conversations.
pub(crate) fn sync_monitor(app: &App, conn: &mut Conn) {
    if !conn.can_monitor() {
        return;
    }
    conn.online.clear();
    let nicks: Vec<String> = monitored(app, conn).into_values().collect();
    for line in monitor_commands(&nicks, limit(conn)) {
        conn.send(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_by_bytes_and_respects_the_server_limit() {
        let nicks: Vec<String> = (0..100).map(|i| format!("nickname{i:03}")).collect();
        let cmds = monitor_commands(&nicks, usize::MAX);
        assert!(cmds.len() > 1);
        assert!(cmds.iter().all(|c| c.len() - "MONITOR + ".len() <= 400));
        assert_eq!(cmds.iter().map(|c| c.matches(',').count() + 1).sum::<usize>(), 100);
        assert_eq!(monitor_commands(&nicks, 3), vec!["MONITOR + nickname000,nickname001,nickname002"]);
        assert!(monitor_commands(&[], usize::MAX).is_empty());
    }
}
