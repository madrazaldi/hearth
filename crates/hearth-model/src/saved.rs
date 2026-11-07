//! Saved buffers: which channels and queries to reopen after a restart.

use crate::buffer::{Buffer, BufferKind};
use crate::effect::{Effect, Reply};
use crate::state::{App, Pending};
use crate::types::{SavedBuffer, ServerConfig};
use crate::Conn;

/// Queries, plus channels we mean to be in.
pub fn saved_list(conn: &Conn) -> Vec<SavedBuffer> {
    conn.buffers_iter()
        .filter(|b| b.kind == BufferKind::Query || (b.kind == BufferKind::Channel && b.want_joined))
        .map(|b| SavedBuffer {
            name: b.name.clone(),
            kind: if b.kind == BufferKind::Query { "query".into() } else { "channel".into() },
            key: b.join_key.clone(),
        })
        .collect()
}

/// JOIN commands for `chans`, ten per line.
pub fn rejoin_commands(chans: &[(&str, &str)]) -> Vec<String> {
    chans
        .chunks(10)
        .map(|group| {
            // Keyed channels must come first so keys line up by position.
            let keyed: Vec<_> = group.iter().filter(|(_, k)| !k.is_empty()).collect();
            let names = keyed.iter().map(|(n, _)| *n).chain(group.iter().filter(|(_, k)| k.is_empty()).map(|(n, _)| *n)).collect::<Vec<_>>().join(",");
            if keyed.is_empty() {
                format!("JOIN {names}")
            } else {
                format!("JOIN {names} {}", keyed.iter().map(|(_, k)| *k).collect::<Vec<_>>().join(","))
            }
        })
        .collect()
}

/// The JOIN lines for channels we meant to be in that the server config won't join itself.
pub(crate) fn rejoin_lines(conn: &Conn, cfg: Option<&ServerConfig>) -> Vec<String> {
    let auto: Vec<String> = cfg.map(|c| c.autojoin.iter().map(|a| conn.fold(a.trim())).collect()).unwrap_or_default();
    let chans: Vec<&Buffer> = conn.buffers_iter().filter(|b| b.kind == BufferKind::Channel && b.want_joined && !auto.contains(&b.key())).collect();
    let pairs: Vec<(&str, &str)> = chans.iter().map(|b| (b.name.as_str(), b.join_key.as_str())).collect();
    rejoin_commands(&pairs)
}

pub(crate) fn restore_buffers(app: &mut App, conn: &mut Conn) {
    let req = app.env.next_req();
    app.pending.insert(req, Pending::Restore { conn: conn.id.clone() });
    app.env.emit(Effect::LoadBuffers { req, conn: conn.id.clone() });
}

/// The answer to `LoadBuffers`: reopen what was saved, then finish connecting.
pub(crate) fn restore_done(app: &mut App, conn_id: &str, result: Result<Reply, String>) {
    app.with_conn(conn_id, |app, conn| {
        if let Ok(Reply::Buffers(saved)) = result {
            for s in saved {
                // Bouncers keep their own channel list; only restore conversations there.
                if s.kind == "channel" && conn.netid.is_some() {
                    continue;
                }
                let kind = if s.kind == "channel" { BufferKind::Channel } else { BufferKind::Query };
                let key = conn.ensure(&s.name, Some(kind));
                if kind == BufferKind::Channel {
                    let b = conn.buf_mut(&key).unwrap();
                    b.want_joined = true;
                    b.join_key = s.key;
                } else if conn.buf(&key).is_some_and(|b| b.lines().is_empty()) {
                    crate::history::fetch_history(app, conn, &key, crate::history::HistMode::Latest);
                }
                app.maybe_restore_active(conn, &key);
            }
        }
        conn.buffers_loaded = true;
        app.finish_connected(conn);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejoins_ten_per_line_with_keyed_channels_first() {
        let cmds = rejoin_commands(&[("#a", ""), ("#b", "k1"), ("#c", "k2")]);
        assert_eq!(cmds, vec!["JOIN #b,#c,#a k1,k2"]);
        let many: Vec<(&str, &str)> = (0..12).map(|_| ("#x", "")).collect();
        assert_eq!(rejoin_commands(&many).len(), 2);
        assert!(rejoin_commands(&[]).is_empty());
    }
}
