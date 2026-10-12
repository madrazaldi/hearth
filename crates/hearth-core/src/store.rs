//! Local SQLite storage: a message log with full-text search, the buffers each
//! connection had open, and STS policies.
//!
//! Writes go through a dedicated thread that batches them into transactions,
//! so logging never blocks a connection actor. Reads use their own connection
//! (WAL mode lets them run alongside the writer).

use crate::irc::message::Message;
use crate::irc::time::{format_ms, now_ms, parse_ms};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    network TEXT NOT NULL,
    target TEXT NOT NULL,
    time INTEGER NOT NULL,
    msgid TEXT,
    nick TEXT NOT NULL,
    command TEXT NOT NULL,
    text TEXT NOT NULL,
    raw TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_by_target ON messages (network, target, time);
CREATE UNIQUE INDEX IF NOT EXISTS messages_by_msgid ON messages (network, msgid) WHERE msgid IS NOT NULL;
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5 (
    text, content = 'messages', content_rowid = 'id', tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TABLE IF NOT EXISTS buffers (
    network TEXT NOT NULL,
    folded TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    key TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (network, folded)
);
CREATE TABLE IF NOT EXISTS sts (
    host TEXT PRIMARY KEY,
    port INTEGER NOT NULL,
    expires INTEGER NOT NULL
);
";

/// One message to append to the log.
pub struct LogEntry {
    pub network: String,
    /// Case-folded buffer name.
    pub target: String,
    pub msg: Message,
}

enum Write {
    Log(LogEntry),
    Redact { network: String, msgid: String },
    Rename { network: String, from: String, to: String },
    Flush(mpsc::Sender<()>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedBuffer {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub network: String,
    pub target: String,
    pub msg: Message,
}

/// A log search. Every field but `limit` narrows it; without `text` it lists
/// the messages the other filters match.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchQuery {
    pub text: String,
    pub network: Option<String>,
    /// Case-folded buffer name.
    pub target: Option<String>,
    /// Sender nick, compared case-insensitively.
    pub from: Option<String>,
    /// Bounds in ms, exclusive.
    pub before: Option<i64>,
    pub after: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StsPolicy {
    pub port: u16,
    pub expires: i64,
}

pub struct Db {
    read: Mutex<Connection>,
    writes: mpsc::Sender<Write>,
    enabled: AtomicBool,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let writer = Connection::open(path)?;
        writer.execute_batch(SCHEMA)?;
        let read = Connection::open(path)?;
        Ok(Self::with(writer, read))
    }

    /// A private in-memory database, for tests.
    pub fn memory() -> Result<Db> {
        // Two handles on one named shared-cache database.
        let uri = format!("file:hearth-{}?mode=memory&cache=shared", uuid::Uuid::new_v4().simple());
        let flags = rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_URI;
        let writer = Connection::open_with_flags(&uri, flags)?;
        writer.execute_batch(SCHEMA)?;
        let read = Connection::open_with_flags(&uri, flags)?;
        Ok(Self::with(writer, read))
    }

    fn with(writer: Connection, read: Connection) -> Db {
        let (tx, rx) = mpsc::channel::<Write>();
        std::thread::Builder::new()
            .name("hearth-db".into())
            .spawn(move || write_loop(writer, rx))
            .expect("spawn db thread");
        Db { read: Mutex::new(read), writes: tx, enabled: AtomicBool::new(true) }
    }

    pub fn set_logging(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn logging(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn log(&self, entry: LogEntry) {
        if self.logging() {
            let _ = self.writes.send(Write::Log(entry));
        }
    }

    pub fn redact(&self, network: &str, msgid: &str) {
        let _ = self.writes.send(Write::Redact { network: network.into(), msgid: msgid.into() });
    }

    /// Move a buffer's log to a new (case-folded) name, e.g. after a channel
    /// rename. Queued behind pending writes so none land under the old name.
    pub fn rename_target(&self, network: &str, from: &str, to: &str) {
        if from != to {
            let _ = self.writes.send(Write::Rename { network: network.into(), from: from.into(), to: to.into() });
        }
    }

    /// Block until every queued write is committed.
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if self.writes.send(Write::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }

    /// Up to `limit` messages in `target` before `before` (ms), oldest first.
    pub fn history(&self, network: &str, target: &str, before: Option<i64>, limit: usize) -> Result<Vec<Message>> {
        let db = self.read.lock().unwrap();
        let mut stmt = db.prepare_cached(
            "SELECT raw FROM messages WHERE network = ?1 AND target = ?2 AND time < ?3 ORDER BY time DESC, id DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![network, target, before.unwrap_or(i64::MAX), limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out: Vec<Message> = rows.filter_map(|r| r.ok()).filter_map(|raw| serde_json::from_str(&raw).ok()).collect();
        out.reverse();
        Ok(out)
    }

    /// Messages after `after` (ms), oldest first.
    pub fn history_after(&self, network: &str, target: &str, after: i64, limit: usize) -> Result<Vec<Message>> {
        let db = self.read.lock().unwrap();
        let mut stmt = db.prepare_cached(
            "SELECT raw FROM messages WHERE network = ?1 AND target = ?2 AND time > ?3 ORDER BY time ASC, id ASC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![network, target, after, limit as i64], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|raw| serde_json::from_str(&raw).ok()).collect())
    }

    /// Search the log, newest first.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        let fts = fts_query(&q.text);
        if fts.is_none() && (!q.text.trim().is_empty() || (q.from.is_none() && q.target.is_none() && q.before.is_none() && q.after.is_none())) {
            return Ok(Vec::new());
        }
        const FILTERS: &str = "(?2 IS NULL OR m.network = ?2 OR m.network LIKE ?2 || '/%')
               AND (?3 IS NULL OR m.target = ?3)
               AND (?4 IS NULL OR m.nick = ?4 COLLATE NOCASE)
               AND m.time < ?5 AND m.time > ?6";
        let sql = if fts.is_some() {
            format!(
                "SELECT m.network, m.target, m.raw FROM messages_fts f JOIN messages m ON m.id = f.rowid
                 WHERE messages_fts MATCH ?1 AND {FILTERS} ORDER BY m.time DESC LIMIT ?7"
            )
        } else {
            format!(
                "SELECT m.network, m.target, m.raw FROM messages m
                 WHERE ?1 IS NULL AND m.command IN ('PRIVMSG', 'NOTICE') AND {FILTERS} ORDER BY m.time DESC LIMIT ?7"
            )
        };
        let db = self.read.lock().unwrap();
        let mut stmt = db.prepare_cached(&sql)?;
        let args = params![
            fts,
            q.network,
            q.target,
            q.from,
            q.before.unwrap_or(i64::MAX),
            q.after.unwrap_or(i64::MIN),
            q.limit as i64
        ];
        let rows = stmt.query_map(args, |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter_map(|(network, target, raw)| Some(SearchHit { network, target, msg: serde_json::from_str(&raw).ok()? }))
            .collect())
    }

    /// Forget everything logged for a server (and its bouncer networks).
    pub fn forget_server(&self, server: &str) -> Result<()> {
        let db = self.read.lock().unwrap();
        let like = format!("{server}/%");
        db.execute("DELETE FROM messages WHERE network = ?1 OR network LIKE ?2", params![server, like])?;
        db.execute("DELETE FROM buffers WHERE network = ?1 OR network LIKE ?2", params![server, like])?;
        Ok(())
    }

    pub fn clear_logs(&self) -> Result<()> {
        let db = self.read.lock().unwrap();
        db.execute("DELETE FROM messages", [])?;
        db.execute("INSERT INTO messages_fts (messages_fts) VALUES ('rebuild')", [])?;
        db.execute_batch("VACUUM")?;
        Ok(())
    }

    pub fn buffers(&self, network: &str) -> Result<Vec<SavedBuffer>> {
        let db = self.read.lock().unwrap();
        let mut stmt = db.prepare_cached("SELECT name, kind, key FROM buffers WHERE network = ?1 ORDER BY folded")?;
        let rows = stmt.query_map([network], |r| Ok(SavedBuffer { name: r.get(0)?, kind: r.get(1)?, key: r.get(2)? }))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Replace the saved buffer list for a connection. `fold` case-folds names.
    pub fn save_buffers(&self, network: &str, buffers: &[SavedBuffer], fold: impl Fn(&str) -> String) -> Result<()> {
        let mut db = self.read.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM buffers WHERE network = ?1", [network])?;
        for b in buffers {
            tx.execute(
                "INSERT OR REPLACE INTO buffers (network, folded, name, kind, key) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![network, fold(&b.name), b.name, b.kind, b.key],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn sts_policy(&self, host: &str) -> Option<StsPolicy> {
        let db = self.read.lock().unwrap();
        let row: Option<(i64, i64)> = db
            .query_row("SELECT port, expires FROM sts WHERE host = ?1", [host.to_ascii_lowercase()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()
            .ok()
            .flatten();
        let (port, expires) = row?;
        (expires > now_ms()).then_some(StsPolicy { port: port as u16, expires })
    }

    /// Record (or with `duration_secs == 0`, drop) a host's STS policy.
    pub fn set_sts_policy(&self, host: &str, port: u16, duration_secs: i64) {
        let db = self.read.lock().unwrap();
        let host = host.to_ascii_lowercase();
        let res = if duration_secs <= 0 {
            db.execute("DELETE FROM sts WHERE host = ?1", [host])
        } else {
            db.execute(
                "INSERT OR REPLACE INTO sts (host, port, expires) VALUES (?1, ?2, ?3)",
                params![host, port, now_ms() + duration_secs * 1000],
            )
        };
        if let Err(e) = res {
            log::warn!("saving STS policy failed: {e}");
        }
    }
}

fn write_loop(mut db: Connection, rx: mpsc::Receiver<Write>) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while batch.len() < 500 {
            match rx.try_recv() {
                Ok(w) => batch.push(w),
                Err(_) => break,
            }
        }
        let mut acks = Vec::new();
        batch.retain(|w| match w {
            Write::Flush(ack) => {
                acks.push(ack.clone());
                false
            }
            _ => true,
        });
        if let Err(e) = apply(&mut db, batch) {
            log::error!("writing message log failed: {e}");
        }
        for ack in acks {
            let _ = ack.send(());
        }
    }
}

fn apply(db: &mut Connection, batch: Vec<Write>) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO messages (network, target, time, msgid, nick, command, text, raw)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut redact = tx.prepare_cached("DELETE FROM messages WHERE network = ?1 AND msgid = ?2")?;
        let mut rename = tx.prepare_cached("UPDATE messages SET target = ?3 WHERE network = ?1 AND target = ?2")?;
        for w in batch {
            match w {
                Write::Log(mut e) => {
                    let time = e.msg.tag("time").and_then(parse_ms).unwrap_or_else(now_ms);
                    e.msg.tags.entry("time".into()).or_insert_with(|| format_ms(time));
                    let msgid = e.msg.tag("msgid").map(str::to_string);
                    let nick = e.msg.source.as_ref().map(|s| s.nick.clone()).unwrap_or_default();
                    let text = searchable_text(&e.msg);
                    let raw = serde_json::to_string(&e.msg).unwrap_or_default();
                    insert.execute(params![e.network, e.target, time, msgid, nick, e.msg.command, text, raw])?;
                }
                Write::Redact { network, msgid } => {
                    redact.execute(params![network, msgid])?;
                }
                Write::Rename { network, from, to } => {
                    rename.execute(params![network, from, to])?;
                }
                Write::Flush(_) => {}
            }
        }
    }
    tx.commit()
}

/// The plain text of a message body, for indexing.
fn searchable_text(m: &Message) -> String {
    match m.command.as_str() {
        "PRIVMSG" | "NOTICE" => {
            let body = m.param(1);
            let body = body
                .strip_prefix("\x01ACTION ")
                .map(|b| b.trim_end_matches('\x01'))
                .unwrap_or(body);
            strip_formatting(body)
        }
        "TOPIC" => strip_formatting(m.param(1)),
        "PART" | "KICK" => strip_formatting(m.params.last().map(String::as_str).unwrap_or("")),
        _ => String::new(),
    }
}

/// Remove mIRC formatting codes.
pub fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x02' | '\x0f' | '\x11' | '\x16' | '\x1d' | '\x1e' | '\x1f' => {}
            '\x03' => {
                let mut fg = false;
                for _ in 0..2 {
                    if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                        chars.next();
                        fg = true;
                    }
                }
                let mut look = chars.clone();
                if fg && look.next() == Some(',') && look.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                    for _ in 0..2 {
                        if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                            chars.next();
                        }
                    }
                }
            }
            '\x04' => {
                let hex = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let snapshot = it.clone();
                    let got: String = snapshot.take(6).collect();
                    if got.len() == 6 && got.chars().all(|c| c.is_ascii_hexdigit()) {
                        for _ in 0..6 {
                            it.next();
                        }
                        true
                    } else {
                        false
                    }
                };
                if hex(&mut chars) {
                    let mut look = chars.clone();
                    if look.next() == Some(',') {
                        let rest: String = look.take(6).collect();
                        if rest.len() == 6 && rest.chars().all(|c| c.is_ascii_hexdigit()) {
                            chars.next();
                            hex(&mut chars);
                        }
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Turn user input into an FTS5 query: every word must match, the last one as
/// a prefix so results appear while typing.
fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '#' && c != '@'))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut q = words.join(" ");
    q.push('*');
    Some(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn privmsg(time: &str, nick: &str, target: &str, text: &str, msgid: Option<&str>) -> Message {
        let mut raw = format!("@time={time}");
        if let Some(id) = msgid {
            raw.push_str(&format!(";msgid={id}"));
        }
        raw.push_str(&format!(" :{nick}!u@h PRIVMSG {target} :{text}"));
        Message::parse(&raw).unwrap()
    }

    fn log(db: &Db, target: &str, msg: Message) {
        db.log(LogEntry { network: "srv".into(), target: target.into(), msg });
    }

    #[test]
    fn logs_pages_and_searches() {
        let db = Db::memory().unwrap();
        log(&db, "#a", privmsg("2026-01-01T00:00:01.000Z", "ann", "#a", "hello \x02world\x02", Some("m1")));
        log(&db, "#a", privmsg("2026-01-01T00:00:02.000Z", "bob", "#a", "\x01ACTION waves at everyone\x01", Some("m2")));
        log(&db, "#a", privmsg("2026-01-01T00:00:02.000Z", "bob", "#a", "duplicate id", Some("m2")));
        log(&db, "#b", privmsg("2026-01-01T00:00:03.000Z", "cat", "#b", "worldly things", None));
        db.flush();

        let all = db.history("srv", "#a", None, 10).unwrap();
        assert_eq!(all.len(), 2, "duplicate msgid is ignored");
        assert_eq!(all[0].tag("msgid"), Some("m1"));

        let before = db.history("srv", "#a", parse_ms("2026-01-01T00:00:02.000Z"), 10).unwrap();
        assert_eq!(before.len(), 1);
        let after = db.history_after("srv", "#a", parse_ms("2026-01-01T00:00:01.000Z").unwrap(), 10).unwrap();
        assert_eq!(after.len(), 1);

        let hits = db.search(&text("worl")).unwrap();
        assert_eq!(hits.len(), 2, "prefix match across buffers, formatting stripped");
        assert_eq!(hits[0].target, "#b", "newest first");
        let scoped = SearchQuery { network: Some("srv".into()), target: Some("#a".into()), ..text("waves") };
        assert_eq!(db.search(&scoped).unwrap().len(), 1);
        assert!(db.search(&text("   ")).unwrap().is_empty());
        assert!(db.search(&text("\"unbalanced")).is_ok());

        db.redact("srv", "m1");
        db.flush();
        assert_eq!(db.history("srv", "#a", None, 10).unwrap().len(), 1);
        assert!(db.search(&text("hello")).unwrap().is_empty());
    }

    fn text(t: &str) -> SearchQuery {
        SearchQuery { text: t.into(), limit: 10, ..Default::default() }
    }

    #[test]
    fn search_filters() {
        let db = Db::memory().unwrap();
        log(&db, "#a", privmsg("2026-01-01T00:00:01.000Z", "Ann", "#a", "deploy one", None));
        log(&db, "#a", privmsg("2026-01-01T00:00:02.000Z", "bob", "#a", "deploy two", None));
        log(&db, "#b", privmsg("2026-01-01T00:00:03.000Z", "ann", "#b", "deploy three", None));
        log(&db, "#b", privmsg("2026-01-01T00:00:04.000Z", "ann", "#b", "unrelated", None));
        db.flush();
        let texts = |q: SearchQuery| db.search(&q).unwrap().iter().map(|h| h.msg.param(1).to_string()).collect::<Vec<_>>();

        assert_eq!(texts(SearchQuery { from: Some("ANN".into()), ..text("deploy") }), ["deploy three", "deploy one"]);
        let t2 = parse_ms("2026-01-01T00:00:02.000Z");
        assert_eq!(texts(SearchQuery { before: t2, ..text("deploy") }), ["deploy one"]);
        assert_eq!(texts(SearchQuery { after: t2, ..text("deploy") }), ["deploy three"]);
        // Paging: the next page starts before the oldest hit so far.
        let page = SearchQuery { limit: 2, ..text("deploy") };
        assert_eq!(texts(page.clone()), ["deploy three", "deploy two"]);
        assert_eq!(texts(SearchQuery { before: t2, ..page }), ["deploy one"]);
        // Filters alone list everything they match; no filters at all, nothing.
        assert_eq!(texts(SearchQuery { from: Some("ann".into()), target: Some("#b".into()), ..text("") }), ["unrelated", "deploy three"]);
        assert!(texts(text("")).is_empty());
    }

    #[test]
    fn renamed_channels_keep_their_log() {
        let db = Db::memory().unwrap();
        log(&db, "#old", privmsg("2026-01-01T00:00:01.000Z", "ann", "#old", "before", None));
        db.rename_target("srv", "#old", "#new");
        log(&db, "#new", privmsg("2026-01-01T00:00:02.000Z", "ann", "#new", "after", None));
        db.flush();
        assert!(db.history("srv", "#old", None, 10).unwrap().is_empty());
        let texts: Vec<_> = db.history("srv", "#new", None, 10).unwrap().iter().map(|m| m.param(1).to_string()).collect();
        assert_eq!(texts, ["before", "after"]);
    }

    #[test]
    fn logging_can_be_disabled() {
        let db = Db::memory().unwrap();
        db.set_logging(false);
        log(&db, "#a", privmsg("2026-01-01T00:00:01.000Z", "ann", "#a", "secret", None));
        db.set_logging(true);
        db.flush();
        assert!(db.history("srv", "#a", None, 10).unwrap().is_empty());
    }

    #[test]
    fn saves_buffers_and_sts() {
        let db = Db::memory().unwrap();
        let bufs = vec![
            SavedBuffer { name: "#Rust".into(), kind: "channel".into(), key: "k".into() },
            SavedBuffer { name: "Ann".into(), kind: "query".into(), key: String::new() },
        ];
        db.save_buffers("srv", &bufs, |s| s.to_lowercase()).unwrap();
        let back = db.buffers("srv").unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "#Rust");
        assert_eq!(back[0].key, "k");

        assert!(db.sts_policy("irc.example").is_none());
        db.set_sts_policy("IRC.example", 6697, 3600);
        assert_eq!(db.sts_policy("irc.example").unwrap().port, 6697);
        db.set_sts_policy("irc.example", 6697, 0);
        assert!(db.sts_policy("irc.example").is_none());
    }

    #[test]
    fn strips_formatting() {
        assert_eq!(strip_formatting("\x0304,12red\x03 \x02b\x02 \x04ff0000,00ff00hex"), "red b hex");
        assert_eq!(strip_formatting("\x03,5"), ",5");
    }
}
