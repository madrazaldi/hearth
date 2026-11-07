//! Helpers for tests that drive an [`App`] with raw IRC lines: the
//! equivalents of the TypeScript `connect`, `feed` and `sent`.

use crate::buffer::{BufId, Buffer};
use crate::effect::Effect;
use crate::state::App;
use crate::types::{ConnStatus, IrcEvent, IrcMessage};
use std::collections::BTreeMap;

/// The connection `connect` creates and `feed` writes to.
pub const CONN: &str = "c";

/// A fixed "now" (2026-10-12 15:00 UTC) for tests that pin the clock.
pub const NOW: i64 = 1_791_817_200_000;

/// An app with its clock pinned at [`NOW`].
pub fn new_app() -> App {
    let app = App::new();
    app.set_now(NOW);
    app
}

/// Parse a raw IRC line.
pub fn parse(line: &str) -> IrcMessage {
    IrcMessage::parse(line).unwrap_or_else(|| panic!("unparseable line: {line:?}"))
}

#[derive(Debug, Clone)]
pub struct ConnectOpts<'a> {
    pub id: &'a str,
    pub nick: &'a str,
    pub caps: &'a [&'a str],
    pub cap_values: &'a [(&'a str, &'a str)],
    pub isupport: &'a [(&'a str, &'a str)],
}

impl Default for ConnectOpts<'_> {
    fn default() -> Self {
        ConnectOpts { id: CONN, nick: "me", caps: &[], cap_values: &[], isupport: &[] }
    }
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Connect a fresh server as `me` with the given caps and extra ISUPPORT.
pub fn connect(app: &mut App, caps: &[&str], isupport: &[(&str, &str)]) {
    connect_with(app, ConnectOpts { caps, isupport, ..ConnectOpts::default() });
}

pub fn connect_with(app: &mut App, o: ConnectOpts) {
    let mut isupport = map(&[("CHANTYPES", "#"), ("PREFIX", "(ov)@+"), ("CASEMAPPING", "ascii")]);
    isupport.extend(map(o.isupport));
    let conn = o.id.to_string();
    app.handle_events(&[
        IrcEvent::ConnAdded { conn: conn.clone(), server: conn.clone(), parent: None, netid: None },
        IrcEvent::Registered { conn: conn.clone(), nick: o.nick.into(), caps: o.caps.iter().map(|c| c.to_string()).collect(), cap_values: map(o.cap_values) },
        IrcEvent::Isupport { conn: conn.clone(), isupport },
        IrcEvent::Status { conn, status: ConnStatus::Connected, error: None, retry_in: None },
    ]);
}

/// Feed raw lines to connection [`CONN`].
pub fn feed(app: &mut App, lines: &[&str]) {
    feed_on(app, CONN, lines);
}

pub fn feed_on(app: &mut App, conn: &str, lines: &[&str]) {
    let events: Vec<IrcEvent> = lines.iter().map(|l| IrcEvent::Message { conn: conn.into(), msg: parse(l) }).collect();
    app.handle_events(&events);
}

/// Lines the app has sent since the last call. Effects that are not sends
/// stay queued.
pub fn sent(app: &mut App) -> Vec<String> {
    let (sends, rest): (Vec<_>, Vec<_>) = app.take_effects().into_iter().partition(|e| matches!(e, Effect::Send { .. }));
    for e in rest {
        app.env().emit(e);
    }
    sends
        .into_iter()
        .flat_map(|e| match e {
            Effect::Send { lines, .. } => lines,
            _ => Vec::new(),
        })
        .collect()
}

/// The queued requests (effects that expect an answer), oldest first. Other
/// effects stay queued.
pub fn requests(app: &mut App) -> Vec<Effect> {
    let (reqs, rest): (Vec<_>, Vec<_>) = app.take_effects().into_iter().partition(|e| e.req().is_some());
    for e in rest {
        app.env().emit(e);
    }
    reqs
}

/// The notifications (title, body) the app has asked for since the last call.
pub fn notifications(app: &mut App) -> Vec<(String, String)> {
    let (notes, rest): (Vec<_>, Vec<_>) = app.take_effects().into_iter().partition(|e| matches!(e, Effect::Notify { .. }));
    for e in rest {
        app.env().emit(e);
    }
    notes
        .into_iter()
        .filter_map(|e| match e {
            Effect::Notify { title, body } => Some((title, body)),
            _ => None,
        })
        .collect()
}

/// The id of a buffer of connection [`CONN`] by display name.
pub fn id_of(app: &App, name: &str) -> BufId {
    let c = app.conn(CONN).expect("connect first");
    BufId::new(CONN, c.key_of(name).unwrap_or_else(|| panic!("no buffer {name}")))
}

/// A buffer of connection [`CONN`] by display name.
pub fn buf<'a>(app: &'a App, name: &str) -> &'a Buffer {
    app.conn(CONN).and_then(|c| c.buffer(name)).unwrap_or_else(|| panic!("no buffer {name}"))
}

/// Texts of a buffer's lines.
pub fn texts(app: &App, name: &str) -> Vec<String> {
    buf(app, name).lines().iter().map(|l| l.text.clone()).collect()
}

/// Texts of the server buffer's lines.
pub fn server_texts(app: &App) -> Vec<String> {
    app.conn(CONN).expect("connect first").server.lines().iter().map(|l| l.text.clone()).collect()
}

/// Move the pinned clock forward and run due timers.
pub fn advance(app: &mut App, ms: i64) {
    app.set_now(app.now() + ms);
    app.tick();
}

/// Queued effects of every kind, leaving the queue empty.
pub fn effects(app: &mut App) -> Vec<Effect> {
    app.take_effects()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connects_and_records_sends() {
        let mut app = new_app();
        connect(&mut app, &["server-time"], &[]);
        feed(&mut app, &[":me!u@h JOIN #a"]);
        assert!(app.conn(CONN).unwrap().buffer("#a").is_some());
        let id = app.conn(CONN).unwrap().buf_id("#a");
        app.send_message(&id, "hi", crate::state::SendKind::Privmsg, false, None);
        assert!(sent(&mut app).iter().any(|l| l.contains("PRIVMSG #a :hi")));
        assert!(sent(&mut app).is_empty());
    }
}
