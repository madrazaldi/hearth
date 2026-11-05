//! Turns IRC messages from the server into state changes and buffer lines.

use crate::buffer::{BufferKind, Member};
use crate::conn::{ChannelInfo, ChannelList};
use crate::effect::Secret;
use crate::env::Timer;
use crate::history;
use crate::irc::{msg_time, parse_attrs, parse_ctcp, parse_mask, parse_time};
use crate::state::{App, ToastAction, ToastActionKind, ToastKind};
use crate::types::{IrcMessage, Line, LineType};
use crate::Conn;
use std::collections::BTreeMap;

/// The fields every line built from a message shares.
struct Base {
    time: i64,
    nick: String,
    is_self: bool,
    msgid: Option<String>,
    account: Option<String>,
    reply_to: Option<String>,
}

impl Base {
    fn of(app: &App, conn: &Conn, m: &IrcMessage) -> Base {
        let nick = m.source.as_ref().map(|s| s.nick.clone()).unwrap_or_default();
        Base {
            time: msg_time(&m.tags, app.now()),
            is_self: conn.is_me(&nick),
            nick,
            msgid: m.tags.get("msgid").cloned(),
            account: m.tags.get("account").cloned(),
            reply_to: m.tags.get("+draft/reply").cloned(),
        }
    }

    fn line(&self, app: &App, kind: LineType) -> Line {
        let mut l = app.env.line(kind);
        l.time = self.time;
        l.nick = self.nick.clone();
        l.is_self = self.is_self;
        l.msgid = self.msgid.clone();
        l.account = self.account.clone();
        l.reply_to = self.reply_to.clone();
        l
    }

    /// Like [`Base::line`] without the sender, for server-voiced lines.
    fn bare(&self, app: &App, kind: LineType, text: impl Into<String>) -> Line {
        let mut l = self.line(app, kind);
        l.nick.clear();
        l.text = text.into();
        l
    }
}

fn put(conn: &mut Conn, key: &str, line: Line) {
    if let Some(b) = conn.buf_mut(key) {
        b.push(line);
    }
}

fn msg_body(m: &IrcMessage) -> Option<(LineType, String)> {
    let raw = m.param(1);
    if let Some(ctcp) = parse_ctcp(raw) {
        return (ctcp.command == "ACTION").then_some((LineType::Action, ctcp.args));
    }
    Some((if m.command == "NOTICE" { LineType::Notice } else { LineType::Privmsg }, raw.to_string()))
}

/// Build a line for a message replayed from history (server or local log).
pub(crate) fn history_line(app: &App, conn: &Conn, m: &IrcMessage) -> Option<Line> {
    let base = Base::of(app, conn, m);
    let host =
        || format!("{}@{}", m.source.as_ref().and_then(|s| s.user.as_deref()).unwrap_or(""), m.source.as_ref().and_then(|s| s.host.as_deref()).unwrap_or(""));
    match m.command.as_str() {
        "PRIVMSG" | "NOTICE" => {
            if app.is_ignored(conn, m.source.as_ref()) {
                return None;
            }
            let (kind, text) = msg_body(m)?;
            let mut l = base.line(app, kind);
            l.highlight = !base.is_self && app.is_highlight(conn, m.source.as_ref(), &text);
            l.text = text;
            Some(l)
        }
        "JOIN" => {
            let mut l = base.line(app, LineType::Join);
            l.data = Some(BTreeMap::from([("host".to_string(), host())]));
            Some(l)
        }
        "PART" => {
            let mut l = base.line(app, LineType::Part);
            l.text = m.param(1).into();
            Some(l)
        }
        "QUIT" => {
            let mut l = base.line(app, LineType::Quit);
            l.text = m.param(0).into();
            Some(l)
        }
        "KICK" => {
            let mut l = base.line(app, LineType::Kick);
            l.target = m.params.get(1).cloned();
            l.text = m.param(2).into();
            Some(l)
        }
        "NICK" => {
            let mut l = base.line(app, LineType::Nick);
            l.target = m.params.first().cloned();
            Some(l)
        }
        "TOPIC" => {
            let mut l = base.line(app, LineType::Topic);
            l.text = m.param(1).into();
            Some(l)
        }
        "MODE" => {
            let mut l = base.line(app, LineType::Mode);
            l.text = m.params.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
            Some(l)
        }
        _ => None,
    }
}

/// Walk a channel mode string: each mode letter with its argument, if it takes one.
fn walk_modes(conn: &Conn, modestr: &str, args: &[String]) -> Vec<(bool, char, Option<String>)> {
    let chanmodes = conn.isupport.get("CHANMODES").map(String::as_str).unwrap_or("beI,k,l,imnpst");
    let mut parts = chanmodes.split(',');
    let (a, b, c) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let pmodes = &conn.prefix().modes;
    let mut adding = true;
    let mut ai = 0;
    let mut out = Vec::new();
    for ch in modestr.chars() {
        if ch == '+' || ch == '-' {
            adding = ch == '+';
            continue;
        }
        let takes = pmodes.contains(ch) || a.contains(ch) || b.contains(ch) || (c.contains(ch) && adding);
        let arg = if takes {
            ai += 1;
            args.get(ai - 1).cloned()
        } else {
            None
        };
        out.push((adding, ch, arg));
    }
    out
}

pub(crate) fn handle_message(app: &mut App, conn: &mut Conn, m: &IrcMessage) {
    let src = m.source.as_ref();
    let p = &m.params;
    let base = Base::of(app, conn, m);
    let (nick, is_self, time) = (base.nick.clone(), base.is_self, base.time);

    match m.command.as_str() {
        "PRIVMSG" | "NOTICE" => on_message(app, conn, m, &base),

        "TAGMSG" => {
            if app.is_ignored(conn, src) {
                return;
            }
            let target = m.param(0);
            let key = if conn.is_channel(target) { conn.key_of(target) } else { conn.key_of(if is_self { target } else { &nick }) };
            let Some(key) = key else { return };
            let emoji = m.tags.get("+draft/react");
            let parent = m.tags.get("+draft/reply");
            let buf = conn.buf_mut(&key).unwrap();
            if let (Some(e), Some(par)) = (emoji, parent) {
                return buf.react(par, e, &nick);
            }
            let Some(typing) = m.tags.get("+typing") else { return };
            if is_self || !app.settings.show_typing {
                return;
            }
            if typing == "done" {
                buf.typing.remove(&nick);
            } else {
                buf.typing.insert(nick.clone(), app.now() + if typing == "active" { 6000 } else { 30000 });
            }
            buf.touch();
        }

        "JOIN" => {
            let chan = m.param(0);
            let key = conn.ensure(chan, Some(BufferKind::Channel));
            if is_self {
                let pending = conn.pending_keys.remove(&key);
                let b = conn.buf_mut(&key).unwrap();
                b.joined = true;
                b.want_joined = true;
                if let Some(k) = pending {
                    b.join_key = k;
                }
                b.clear_members();
                b.touch();
                if app.pending_active.get(&conn.id) == Some(&conn.fold(chan)) {
                    app.pending_active.remove(&conn.id);
                    app.open_in(conn, &key);
                } else {
                    app.maybe_restore_active(conn, &key);
                }
                conn.save_buffers_soon();
                let mode = if conn.buf(&key).is_some_and(|b| !b.lines().is_empty()) { history::HistMode::After } else { history::HistMode::Latest };
                history::fetch_history(app, conn, &key, mode);
                return;
            }
            let account = (conn.has("extended-join") && m.param(1) != "*" && m.params.len() > 1).then(|| m.param(1).to_string());
            let mem = Member {
                nick: nick.clone(),
                prefix: String::new(),
                away: false,
                account,
                user: src.and_then(|s| s.user.clone()),
                host: src.and_then(|s| s.host.clone()),
                realname: m.params.get(2).cloned(),
            };
            let mut l = base.line(app, LineType::Join);
            l.data = Some(BTreeMap::from([(
                "host".to_string(),
                format!("{}@{}", src.and_then(|s| s.user.as_deref()).unwrap_or(""), src.and_then(|s| s.host.as_deref()).unwrap_or("")),
            )]));
            let b = conn.buf_mut(&key).unwrap();
            b.set_member(mem);
            b.push(l);
        }

        "PART" => {
            let Some(key) = conn.key_of(m.param(0)) else { return };
            let line = {
                let mut l = base.line(app, LineType::Part);
                l.text = m.param(1).into();
                l
            };
            let b = conn.buf_mut(&key).unwrap();
            if is_self {
                b.joined = false;
                b.want_joined = false;
                b.clear_members();
                b.touch();
                conn.save_buffers_soon();
            } else {
                b.remove_member(&nick);
            }
            put(conn, &key, line);
        }

        "KICK" => {
            let Some(key) = conn.key_of(m.param(0)) else { return };
            let victim = m.param(1).to_string();
            let me = conn.is_me(&victim);
            let mut l = base.line(app, LineType::Kick);
            l.target = Some(victim.clone());
            l.text = m.param(2).into();
            l.highlight = me;
            let b = conn.buf_mut(&key).unwrap();
            if me {
                b.joined = false;
                b.clear_members();
                b.touch();
            } else {
                b.remove_member(&victim);
            }
            b.push(l);
        }

        "QUIT" => {
            let fk = conn.fold(&nick);
            conn.metadata.remove(&fk);
            for key in conn.keys() {
                let line = {
                    let mut l = base.line(app, LineType::Quit);
                    l.text = m.param(0).into();
                    l
                };
                let is_query_peer = conn.buf(&key).is_some_and(|b| b.kind == BufferKind::Query && conn.fold(&b.name) == fk);
                let b = conn.buf_mut(&key).unwrap();
                if b.remove_member(&nick) || is_query_peer {
                    b.push(line);
                    if b.typing.remove(&nick).is_some() {
                        b.touch();
                    }
                }
            }
        }

        "NICK" => {
            let next = m.param(0).to_string();
            if is_self {
                conn.nick = next.clone();
            }
            let (old_fold, new_fold) = (conn.fold(&nick), conn.fold(&next));
            if let Some(meta) = conn.metadata.remove(&old_fold) {
                conn.metadata.insert(new_fold, meta);
            }
            // Keys snapshot: renaming a query re-files it under another key.
            for key in conn.keys() {
                let (mem, is_query, is_any_query) = {
                    let b = conn.buf(&key).unwrap();
                    (b.member(&nick).cloned(), b.kind == BufferKind::Query && conn.fold(&b.name) == old_fold, b.kind == BufferKind::Query)
                };
                let free = conn.key_of(&next).is_none_or(|k| k == key);
                let has_mem = mem.is_some();
                if let Some(mem) = mem {
                    let b = conn.buf_mut(&key).unwrap();
                    b.remove_member(&nick);
                    b.set_member(Member { nick: next.clone(), ..mem });
                }
                if has_mem || is_query || (is_self && is_any_query) {
                    let mut l = base.line(app, LineType::Nick);
                    l.target = Some(next.clone());
                    put(conn, &key, l);
                }
                // A case-only change folds to the same key, so the buffer is still "free".
                if is_query && free {
                    if let Some(new_key) = conn.rename_buffer(&key, &next) {
                        app.rekey(&conn.id, &key, &new_key);
                    }
                    conn.save_buffers_soon();
                }
            }
            if is_self {
                let mut l = base.line(app, LineType::Nick);
                l.target = Some(next);
                conn.server.push(l);
            }
        }

        "MODE" => on_mode(app, conn, m, &base),

        "TOPIC" => {
            let Some(key) = conn.key_of(m.param(0)) else { return };
            let mut l = base.line(app, LineType::Topic);
            l.text = m.param(1).into();
            let b = conn.buf_mut(&key).unwrap();
            b.topic = m.param(1).into();
            b.topic_by = nick;
            b.topic_time = time;
            b.push(l);
        }

        "INVITE" => {
            if app.is_ignored(conn, src) {
                return;
            }
            let me = conn.is_me(m.param(0));
            let mut l = base.line(app, LineType::Invite);
            l.target = Some(m.param(1).into());
            l.text = format!("invited {} to {}", if me { "you" } else { m.param(0) }, m.param(1));
            l.highlight = me;
            let key = app.here_in(conn, None);
            put(conn, &key, l);
        }

        "AWAY" => {
            let away = !m.param(0).is_empty();
            for key in conn.keys() {
                let b = conn.buf_mut(&key).unwrap();
                if let Some(mem) = b.member(&nick).cloned() {
                    if mem.away != away {
                        b.set_member(Member { away, ..mem });
                    }
                }
            }
            if is_self {
                conn.away = away;
            }
        }

        "ACCOUNT" => {
            let v = m.param(0);
            update_member(conn, &nick, |mem| mem.account = (v != "*").then(|| v.to_string()));
        }
        "CHGHOST" => update_member(conn, &nick, |mem| {
            mem.user = Some(m.param(0).into());
            mem.host = Some(m.param(1).into());
        }),
        "SETNAME" => update_member(conn, &nick, |mem| mem.realname = Some(m.param(0).into())),

        "MARKREAD" => {
            let ts = m.param(1).strip_prefix("timestamp=").and_then(parse_time);
            if let (Some(key), Some(ts)) = (conn.key_of(m.param(0)), ts) {
                let b = conn.buf_mut(&key).unwrap();
                if ts > b.read_marker {
                    b.set_read_marker(ts);
                }
            }
        }

        "BOUNCER" => {
            if m.param(0) == "NETWORK" && conn.is_control() {
                let id = m.param(1).to_string();
                if m.param(2) == "*" {
                    conn.networks.remove(&id);
                } else {
                    conn.networks.entry(id).or_default().extend(parse_attrs(m.param(2)));
                }
            } else if m.param(0) == "ADDNETWORK" {
                app.toast("Network added", ToastKind::Info);
            }
        }

        "REDACT" => {
            let t = m.param(0);
            let key = if conn.is_channel(t) { conn.key_of(t) } else { conn.key_of(if is_self { t } else { &nick }) };
            if let Some(key) = key {
                conn.buf_mut(&key).unwrap().redact(m.param(1));
            }
        }

        "ERROR" => {
            let text = m.params.first().cloned().unwrap_or_else(|| "Connection error".into());
            conn.server.push(base.bare(app, LineType::Error, text));
        }

        "FAIL" | "WARN" | "NOTE" => on_standard_reply(app, conn, m, &base),
        "RENAME" => on_rename(app, conn, m, &base),

        "METADATA" => {
            // METADATA <target> <key> <visibility> [:<value>]; no value means unset.
            if p.len() >= 3 {
                conn.set_metadata(&p[0], &p[1], p.get(3).map(String::as_str));
            }
        }

        "REGISTER" | "VERIFY" => on_registration(app, conn, m, &base),

        "PING" | "PONG" | "CAP" | "AUTHENTICATE" | "BATCH" | "ACK" => {}

        cmd if cmd.len() == 3 && cmd.bytes().all(|b| b.is_ascii_digit()) => on_numeric(app, conn, m, time),
        _ => {}
    }
}

fn update_member(conn: &mut Conn, nick: &str, f: impl Fn(&mut Member)) {
    for key in conn.keys() {
        let b = conn.buf_mut(&key).unwrap();
        if let Some(mut mem) = b.member(nick).cloned() {
            f(&mut mem);
            b.set_member(mem);
        }
    }
}

fn ctcp_reply(cmd: &str, args: &str) -> Option<String> {
    match cmd {
        "VERSION" => Some("Hearth - a modern IRC client".into()),
        "PING" => Some(args.chars().take(64).collect()),
        "TIME" => Some(chrono::Local::now().format("%a %b %d %Y %H:%M:%S GMT%z").to_string()),
        "CLIENTINFO" => Some("ACTION CLIENTINFO PING TIME VERSION".into()),
        _ => None,
    }
}

fn on_message(app: &mut App, conn: &mut Conn, m: &IrcMessage, base: &Base) {
    let nick = base.nick.clone();
    let src = m.source.as_ref();
    let statusmsg = conn.isupport.get("STATUSMSG").cloned().unwrap_or_default();
    let mut target = m.param(0);
    while let Some(c) = target.chars().next() {
        if statusmsg.contains(c) && conn.is_channel(&target[c.len_utf8()..]) {
            target = &target[c.len_utf8()..];
        } else {
            break;
        }
    }
    let raw = m.param(1);
    let ctcp = parse_ctcp(raw);
    let is_notice = m.command == "NOTICE";

    // Server-originated notices: no source, a server name as source, or pre-registration.
    let from_server = match src {
        None => true,
        Some(s) => target == "*" || (s.user.is_none() && (s.nick.contains('.') || conn.nick.is_empty())),
    };
    if from_server && !conn.is_channel(target) {
        conn.server.push({
            let mut l = base.line(app, LineType::Notice);
            l.text = raw.into();
            l
        });
        return;
    }
    if app.is_ignored(conn, src) {
        return;
    }

    if let Some(c) = ctcp.as_ref().filter(|c| c.command != "ACTION") {
        if !is_notice && !base.is_self {
            // Answer at most a few CTCPs per 10s: replies share our send queue.
            let now = app.now();
            conn.ctcp_times.retain(|t| now - t < 10_000);
            if conn.ctcp_times.len() >= 3 {
                if now - conn.ctcp_warned > 60_000 {
                    conn.ctcp_warned = now;
                    let mut l = base.line(app, LineType::Info);
                    l.text = "Ignoring a flood of CTCP requests".into();
                    conn.server.push(l);
                }
                return;
            }
            conn.ctcp_times.push(now);
            if let Some(reply) = ctcp_reply(&c.command, &c.args) {
                let sp = if reply.is_empty() { String::new() } else { format!(" {reply}") };
                conn.send(format!("NOTICE {nick} :\x01{}{sp}\x01", c.command));
            }
            let mut l = base.line(app, LineType::Info);
            l.text = format!("CTCP {} request from {nick}", c.command);
            let key = app.here_in(conn, None);
            put(conn, &key, l);
        } else if is_notice {
            let mut l = base.line(app, LineType::Info);
            l.text = format!("CTCP {} reply from {nick}: {}", c.command, c.args);
            let key = app.here_in(conn, None);
            put(conn, &key, l);
        }
        return;
    }

    let kind = if ctcp.is_some() {
        LineType::Action
    } else if is_notice {
        LineType::Notice
    } else {
        LineType::Privmsg
    };
    let text = ctcp.map(|c| c.args).unwrap_or_else(|| raw.to_string());
    let is_chan = conn.is_channel(target);
    #[allow(clippy::needless_late_init)] // the other branch returns early
    let key;
    if is_chan {
        key = conn.ensure(target, Some(BufferKind::Channel));
    } else {
        let peer = if base.is_self { target } else { nick.as_str() };
        let existing = conn.key_of(peer);
        if is_notice && existing.is_none() {
            // Private notices without a conversation land where the user is looking.
            let here = app.here_in(conn, m.tags.get("label").map(String::as_str));
            let mut l = base.line(app, kind);
            l.text = text.clone();
            l.target = base.is_self.then(|| target.to_string());
            l.highlight = !base.is_self;
            put(conn, &here, l);
            if !base.is_self {
                app.maybe_notify_in(conn, &here, &nick, &text, true);
            }
            return;
        }
        key = match existing {
            Some(k) => k,
            None => conn.ensure(peer, Some(BufferKind::Query)),
        };
    }

    let highlight = !base.is_self && app.is_highlight(conn, src, &text);
    let mut l = base.line(app, kind);
    l.text = text.clone();
    l.highlight = highlight;
    let b = conn.buf_mut(&key).unwrap();
    if b.typing.remove(&nick).is_some() {
        b.touch();
    }
    if !is_chan && b.member(&nick).is_none() && !base.is_self {
        b.set_member(Member { nick: nick.clone(), user: src.and_then(|s| s.user.clone()), host: src.and_then(|s| s.host.clone()), ..Member::default() });
    }
    b.push(l);
    if !base.is_self {
        app.maybe_notify_in(conn, &key, &nick, &text, highlight || !is_chan);
    }
}

/// "ACCOUNT_REQUIRED" becomes "Account required", for replies without a description.
fn humanize(code: &str) -> String {
    let s = code.replace('_', " ").to_lowercase();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// IRCv3 standard replies: FAIL/WARN/NOTE <command> <code> [<context>...] :<description>.
/// They go to the buffer that sent the command (by label), else to a buffer
/// named in the context, else where the user is looking.
fn on_standard_reply(app: &mut App, conn: &mut Conn, m: &IrcMessage, base: &Base) {
    let p = &m.params;
    let command = p.first().map(String::as_str).unwrap_or("*");
    let code = m.param(1);
    let context: &[String] = if p.len() > 3 { &p[2..p.len() - 1] } else { &[] };
    let last = if p.len() > 2 { p[p.len() - 1].as_str() } else { "" };
    let desc = if last.is_empty() { humanize(code) } else { last.to_string() };
    let kind = m.command.as_str();
    // Commands whose failures are reported to whoever asked (history requests).
    if matches!(command, "CHATHISTORY" | "MARKREAD" | "SEARCH") {
        return;
    }
    if command == "BOUNCER" && kind == "FAIL" {
        return app.toast(format!("Bouncer: {desc}"), ToastKind::Error);
    }
    if command == "REGISTER" || command == "VERIFY" {
        return on_registration(app, conn, m, base);
    }
    let label = m.tags.get("label").map(String::as_str);
    // Our automatic metadata subscription failing is nothing the user can act on.
    if command == "METADATA" && label.is_none() && ["TOO_MANY_SUBS", "KEY_INVALID", "RATE_LIMITED"].contains(&code) {
        return;
    }

    let about = if label.is_some() { None } else { context.iter().filter(|c| !c.is_empty() && *c != "*").find_map(|c| conn.key_of(c)) };
    let where_ = match (label, about) {
        (Some(_), _) => app.here_in(conn, label),
        (None, Some(k)) => k,
        (None, None) if kind == "NOTE" => String::new(),
        (None, None) => app.here_in(conn, None),
    };
    let what = if command == "*" { "" } else { command };
    let text = match kind {
        "FAIL" if !what.is_empty() => format!("{what} failed: {desc}"),
        "FAIL" => desc,
        "WARN" => format!("Warning{}: {desc}", if what.is_empty() { String::new() } else { format!(" ({what})") }),
        _ => desc,
    };
    let mut l = base.bare(app, if kind == "FAIL" { LineType::Error } else { LineType::Info }, text);
    l.data = Some(BTreeMap::from([("code".to_string(), code.to_string())]));
    put(conn, &where_, l);
}

/// draft/channel-rename: RENAME <old> <new> [:<reason>]. The buffer keeps its history and state.
fn on_rename(app: &mut App, conn: &mut Conn, m: &IrcMessage, base: &Base) {
    let (from, to, reason) = (m.param(0), m.param(1), m.param(2));
    let Some(old_key) = conn.key_of(from) else { return };
    if to.is_empty() {
        return;
    }
    if let Some(clash) = conn.key_of(to).filter(|k| *k != old_key) {
        // A stale buffer under the new name (e.g. parted earlier) gives way.
        let id = conn.buf_id(&clash);
        conn.remove_buffer(&clash);
        if app.active.as_ref() == Some(&id) {
            app.active = Some(conn.buf_id(&old_key));
        }
    }
    let old_pref = conn.buf(&old_key).unwrap().pref_key();
    let new_key = conn.rename_buffer(&old_key, to).unwrap();
    app.rekey(&conn.id, &old_key, &new_key);
    let new_pref = conn.buf(&new_key).unwrap().pref_key();
    if let Some(level) = app.settings.buffer_notify.get(&old_pref).copied() {
        app.update_settings(|s| {
            s.buffer_notify.remove(&old_pref);
            s.buffer_notify.insert(new_pref, level);
        });
    }
    if app.active.as_ref() == Some(&conn.buf_id(&new_key)) {
        app.remember_active_in(conn);
    }
    conn.save_buffers_soon();
    let who = if !base.nick.is_empty() && !base.is_self { format!("{} renamed", base.nick) } else { "Renamed".into() };
    let why = if reason.is_empty() { String::new() } else { format!(" ({reason})") };
    put(conn, &new_key, base.bare(app, LineType::Info, format!("{who} {from} to {to}{why}")));
}

/// draft/account-registration replies:
///   REGISTER SUCCESS|VERIFICATION_REQUIRED <account> :<message>
///   VERIFY SUCCESS <account> :<message>
///   FAIL REGISTER|VERIFY <code> [<account>] :<description>
fn on_registration(app: &mut App, conn: &mut Conn, m: &IrcMessage, base: &Base) {
    let p = &m.params;
    let fail = m.command == "FAIL";
    let command = if fail { m.param(0) } else { m.command.as_str() };
    let status = if fail { m.param(1) } else { m.param(0) };
    let account = if fail { "" } else { m.param(1) };
    let desc = if p.len() > 2 { p[p.len() - 1].as_str() } else { "" };
    let here = app.here_in(conn, m.tags.get("label").map(String::as_str));
    let say = |conn: &mut Conn, app: &App, text: String, kind: LineType| put(conn, &here, base.bare(app, kind, text));

    if fail {
        let what = if command == "VERIFY" { "Verification" } else { "Registration" };
        let why = if desc.is_empty() { humanize(status) } else { desc.to_string() };
        say(conn, app, format!("{what} failed: {why}"), LineType::Error);
        // A wrong code can be retried; a refused registration is over.
        if command == "REGISTER" {
            conn.registration = None;
        }
        return;
    }
    if status == "VERIFICATION_REQUIRED" {
        if let Some(r) = conn.registration.as_mut() {
            r.account = account.to_string();
        }
        let head = if desc.is_empty() { format!("Account {account} needs verifying") } else { desc.to_string() };
        say(conn, app, format!("{head} \u{2014} then type /verify {account} <code>"), LineType::Info);
        return;
    }
    if status != "SUCCESS" {
        return;
    }
    say(conn, app, if command == "VERIFY" { format!("Account {account} verified") } else { format!("Account {account} registered") }, LineType::Info);
    let creds = conn.registration.take();
    // Offer SASL only for a server we connect to directly: a bouncer network's
    // config is the bouncer's own login.
    let direct = conn.netid.is_none() && !conn.is_control() && app.servers.iter().any(|s| s.id == conn.server_id);
    let Some(creds) = creds.filter(|_| direct) else { return };
    let name = if account.is_empty() { creds.account.clone() } else { account.to_string() };
    app.push_toast(
        format!("Registered {name}. Sign in automatically from now on?"),
        ToastKind::Info,
        Some(ToastAction {
            label: "Save login".into(),
            kind: ToastActionKind::SaveLogin { conn: conn.id.clone(), account: name, password: Secret(creds.password.0) },
        }),
    );
}

fn on_mode(app: &mut App, conn: &mut Conn, m: &IrcMessage, base: &Base) {
    let target = m.param(0);
    let modestr = m.param(1);
    let args: &[String] = if m.params.len() > 2 { &m.params[2..] } else { &[] };
    let text = std::iter::once(modestr).chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
    if !conn.is_channel(target) {
        if conn.is_me(target) {
            let mut l = base.line(app, LineType::Mode);
            l.text = text;
            conn.server.push(l);
        }
        return;
    }
    let Some(key) = conn.key_of(target) else { return };
    let prefix = conn.prefix().clone();
    let mut save = false;
    for (adding, ch, arg) in walk_modes(conn, modestr, args) {
        let b = conn.buf_mut(&key).unwrap();
        if let Some(sym) = prefix.symbol_of(ch) {
            if let Some(mem) = arg.as_deref().and_then(|a| b.member(a)).cloned() {
                let mut p = mem.prefix.replacen(sym, "", 1);
                if adding {
                    p.push(sym);
                    let mut chars: Vec<char> = p.chars().collect();
                    chars.sort_by_key(|c| prefix.rank(*c).unwrap_or(usize::MAX));
                    p = chars.into_iter().collect();
                }
                b.set_member(Member { prefix: p, ..mem });
            }
        } else if ch == 'k' {
            // Servers hide the key from non-ops with "*".
            if !adding {
                b.join_key.clear();
            } else if let Some(a) = arg.filter(|a| a != "*") {
                b.join_key = a;
            }
            save = true;
        }
    }
    if save {
        conn.save_buffers_soon();
    }
    let mut l = base.line(app, LineType::Mode);
    l.text = text;
    put(conn, &key, l);
}

/// Error numerics whose first parameter is the channel or nick they concern.
const TARGET_ERRORS: &[&str] = &["401", "403", "404", "405", "406", "442", "467", "471", "473", "474", "475", "477", "478", "482", "489"];

fn on_numeric(app: &mut App, conn: &mut Conn, m: &IrcMessage, time: i64) {
    let p = &m.params;
    let rest: &[String] = if p.is_empty() { &[] } else { &p[1..] };
    let trailing = p.last().cloned().unwrap_or_default();
    let label = m.tags.get("label").map(String::as_str);
    let at = |i: usize| m.param(i);

    let info = |app: &App, conn: &mut Conn, text: String, key: &str, kind: LineType| {
        let mut l = app.env.line(kind);
        l.time = time;
        l.text = text;
        put(conn, key, l);
    };

    match m.command.as_str() {
        "001" => info(app, conn, trailing, "", LineType::Info),
        "002" | "003" | "004" | "005" | "250" | "251" | "252" | "253" | "254" | "255" | "265" | "266" | "396" => {}
        "375" => conn.motd.clear(),
        "372" => {
            let t = trailing.strip_prefix("- ").or_else(|| trailing.strip_prefix('-')).unwrap_or(&trailing).to_string();
            conn.motd.push(t);
        }
        "376" => {
            if !conn.motd.is_empty() {
                let key = if label.is_some() { app.here_in(conn, label) } else { String::new() };
                let mut l = app.env.line(LineType::Info);
                l.time = time;
                l.data = Some(BTreeMap::from([("motd".to_string(), conn.motd.join("\n"))]));
                put(conn, &key, l);
            }
            conn.motd.clear();
        }
        "422" => {}
        "221" => {
            let key = app.here_in(conn, label);
            info(app, conn, format!("Your user modes: {}", at(1)), &key, LineType::Info);
        }
        "324" => {
            if let Some(key) = conn.key_of(at(1)) {
                let modes = p.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
                let args: Vec<String> = p.iter().skip(3).cloned().collect();
                let ops = walk_modes(conn, at(2), &args);
                let b = conn.buf_mut(&key).unwrap();
                b.modes = modes;
                b.touch();
                let mut save = false;
                for (adding, ch, arg) in ops {
                    if let (true, 'k', Some(a)) = (adding, ch, arg) {
                        if a != "*" && b.join_key != a {
                            b.join_key = a;
                            save = true;
                        }
                    }
                }
                if save {
                    conn.save_buffers_soon();
                }
            }
        }
        "329" => {}
        "331" => {
            if let Some(b) = conn.buffer_mut(at(1)) {
                b.topic.clear();
                b.touch();
            }
        }
        "332" => {
            if let Some(b) = conn.buffer_mut(at(1)) {
                b.topic = trailing;
                b.touch();
            }
        }
        "333" => {
            if let Some(b) = conn.buffer_mut(at(1)) {
                b.topic_by = at(2).split('!').next().unwrap_or("").to_string();
                b.topic_time = at(3).parse::<i64>().unwrap_or(0) * 1000;
                b.touch();
            }
        }
        "353" => {
            let Some(key) = conn.key_of(at(2)) else { return };
            let symbols = conn.prefix().symbols.clone();
            let mut acc = conn.buf_mut(&key).unwrap().names_acc.take().unwrap_or_default();
            for entry in trailing.split(' ').filter(|e| !e.is_empty()) {
                let split = entry.char_indices().find(|(_, c)| !symbols.contains(*c)).map_or(entry.len(), |(i, _)| i);
                let (prefix, rest) = entry.split_at(split);
                let mk = parse_mask(rest);
                let prev = conn.buf(&key).unwrap().member(&mk.nick).cloned();
                acc.insert(
                    conn.fold(&mk.nick),
                    Member {
                        nick: mk.nick,
                        prefix: prefix.to_string(),
                        away: prev.as_ref().is_some_and(|m| m.away),
                        user: mk.user,
                        host: mk.host,
                        account: prev.and_then(|m| m.account),
                        realname: None,
                    },
                );
            }
            conn.buf_mut(&key).unwrap().names_acc = Some(acc);
        }
        "366" => {
            if let Some(b) = conn.buffer_mut(at(1)) {
                if let Some(acc) = b.names_acc.take() {
                    b.members = acc;
                    b.touch();
                }
            }
        }
        "301" => {
            let nick = at(1);
            let fk = conn.fold(nick);
            if let Some(w) = conn.whois.get_mut(&fk) {
                w.insert("away".into(), trailing);
            } else if let Some(key) = conn.key_of(nick) {
                info(app, conn, format!("{nick} is away: {trailing}"), &key, LineType::Info);
            }
        }
        "305" => {
            conn.away = false;
            conn.auto_away = false;
            let key = app.here_in(conn, label);
            info(app, conn, "You are no longer marked as away".into(), &key, LineType::Info);
        }
        "306" => {
            conn.away = true;
            let key = app.here_in(conn, label);
            let t = if conn.auto_away { "You have been marked as away (window in the background)" } else { "You have been marked as away" };
            info(app, conn, t.into(), &key, LineType::Info);
        }
        "311" | "314" => {
            let nick = at(1).to_string();
            let w =
                BTreeMap::from([("nick".into(), nick.clone()), ("user".into(), at(2).into()), ("host".into(), at(3).into()), ("realname".into(), trailing)]);
            conn.whois.insert(conn.fold(&nick), w);
        }
        "312" | "313" | "317" | "319" | "320" | "330" | "338" | "378" | "379" | "671" | "276" => {
            let fk = conn.fold(at(1));
            let Some(w) = conn.whois.get_mut(&fk) else {
                let key = app.here_in(conn, label);
                return info(app, conn, rest.join(" "), &key, LineType::Info);
            };
            match m.command.as_str() {
                "312" => {
                    w.insert("server".into(), format!("{} \u{2014} {trailing}", at(2)));
                }
                "313" => {
                    w.insert("oper".into(), trailing);
                }
                "317" => {
                    w.insert("idle".into(), at(2).into());
                    if !at(3).is_empty() && at(3).bytes().all(|b| b.is_ascii_digit()) {
                        w.insert("signon".into(), at(3).into());
                    }
                }
                "319" => {
                    let prev = w.get("channels").cloned().unwrap_or_default();
                    w.insert("channels".into(), format!("{prev} {trailing}").trim().to_string());
                }
                "330" => {
                    w.insert("account".into(), at(2).into());
                }
                "671" | "276" => {
                    w.insert("secure".into(), trailing);
                }
                "338" => {
                    let mid = if p.len() > 3 { p[2..p.len() - 1].join(" ") } else { String::new() };
                    w.insert("actual".into(), mid);
                }
                other => {
                    w.insert(format!("extra{other}"), trailing);
                }
            }
        }
        "318" | "369" => {
            if let Some(w) = conn.whois.remove(&conn.fold(at(1))) {
                let key = app.here_in(conn, label);
                let mut l = app.env.line(LineType::Whois);
                l.time = time;
                l.nick = w.get("nick").cloned().unwrap_or_default();
                l.data = Some(w);
                put(conn, &key, l);
            }
        }
        "341" => {
            let key = conn.key_of(at(2)).unwrap_or_else(|| app.here_in(conn, label));
            info(app, conn, format!("Invited {} to {}", at(1), at(2)), &key, LineType::Info);
        }

        // Channel list
        "321" => {
            conn.list_acc = Some(Vec::new());
            let items = conn.channel_list.take().map(|l| l.items).unwrap_or_default();
            conn.channel_list = Some(ChannelList { loading: true, items, at: app.now() });
        }
        "322" => {
            let acc = conn.list_acc.get_or_insert_with(Vec::new);
            acc.push(ChannelInfo { name: at(1).into(), users: at(2).parse().unwrap_or(0), topic: trailing });
            // Publish progress in steps; a big network lists tens of thousands.
            if acc.len().is_multiple_of(1000) {
                conn.channel_list = Some(ChannelList { loading: true, items: acc.clone(), at: app.now() });
            }
        }
        "323" => {
            conn.channel_list = Some(ChannelList { loading: false, items: conn.list_acc.take().unwrap_or_default(), at: app.now() });
        }

        // MONITOR
        "730" | "731" => {
            let online = m.command == "730";
            for entry in trailing.split(',') {
                let nick = parse_mask(entry.trim()).nick;
                if nick.is_empty() {
                    continue;
                }
                let key = conn.fold(&nick);
                let was = conn.online.insert(key, online);
                if was.is_none() || was == Some(online) || !crate::monitor::is_watched(app, conn, &nick) {
                    continue;
                }
                let text = format!("{nick} is {}", if online { "online" } else { "offline" });
                info(app, conn, text.clone(), "", LineType::Info);
                if let Some(q) = conn.key_of(&nick) {
                    info(app, conn, text, &q, LineType::Info);
                }
            }
        }
        "732" | "733" => {}

        // draft/metadata-2
        "761" => {
            // <client> <target> <key> <visibility> :<value>
            conn.set_metadata(at(1), at(2), Some(m.params.get(4).map(String::as_str).unwrap_or("")));
        }
        "766" => conn.set_metadata(at(1), at(2), None),
        "774" => {
            // RPL_METADATASYNCLATER <client> <target> [<retry after>]: ask again later.
            let secs = at(2).parse::<i64>().ok().filter(|n| *n != 0).unwrap_or(5).clamp(1, 300);
            conn.env.schedule(secs * 1000, Timer::MetadataSync { conn: conn.id.clone(), target: at(1).to_string() });
        }
        "760" | "762" | "770" | "771" | "772" => {}
        "734" => info(app, conn, format!("Can't watch more nicknames: the server allows {}", at(1)), "", LineType::Error),

        "900" => info(app, conn, trailing, "", LineType::Info),
        "903" => {}
        "904" | "905" | "906" | "902" => info(app, conn, format!("SASL: {trailing}"), "", LineType::Error),
        "433" => {
            let key = app.here_in(conn, label);
            info(app, conn, format!("Nickname {} is already in use", at(1)), &key, LineType::Error);
        }

        code => {
            let n: u32 = code.parse().unwrap_or(0);
            if (400..600).contains(&n) {
                // Unlabeled errors about a channel or nick belong with that conversation,
                // not whatever the user happens to be looking at.
                let about = if label.is_none() && TARGET_ERRORS.contains(&code) { conn.key_of(at(1)) } else { None };
                let key = about.unwrap_or_else(|| app.here_in(conn, label));
                let n = rest.len();
                let text = rest.iter().enumerate().filter(|(i, x)| *i + 1 < n || !x.is_empty()).map(|(_, x)| x.as_str()).collect::<Vec<_>>().join(" ");
                return info(app, conn, text, &key, LineType::Error);
            }
            let key = if label.is_some() { app.here_in(conn, label) } else { String::new() };
            info(app, conn, rest.join(" "), &key, LineType::Info);
        }
    }
}
