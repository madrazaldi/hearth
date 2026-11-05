//! Model to view data: what the sidebar, header, member list and message list
//! show, read from the host's `hearth_model::App`. Nothing here mutates the
//! model; pictures that are missing are queued on the host's `Images`.

use super::data::*;
use crate::host::Hearth;
use crate::host::images::Ask;
use hearth_model::format::strip_formatting;
use hearth_model::panels::presence;
use hearth_model::state::AvatarState;
use hearth_model::time::format_full;
use hearth_model::{BufId, Conn};

/// Collect every missing picture first, then let the caller flush the queue once.
pub fn avatar_pic(h: &Hearth, url: Option<String>) -> Pic {
    let Some(url) = url.filter(|_| h.app.settings.load_avatars) else { return Pic(None) };
    match h.app.avatar(&url) {
        AvatarState::Ready(data) => Pic(h.images.get(&url, &data)),
        AvatarState::Loading => Pic(None),
        AvatarState::Unavailable => {
            h.images.ask(Ask::Avatar(url));
            Pic(None)
        }
    }
}

/// The picture a person set with draft/metadata-2, if it has loaded.
pub fn nick_pic(h: &Hearth, conn: &Conn, nick: &str) -> Pic {
    avatar_pic(h, conn.profile(nick).avatar)
}

pub fn sidebar(h: &Hearth) -> SidebarView {
    let app = &h.app;
    let mut networks: Vec<NetworkGroup> = Vec::new();
    for n in app.sidebar() {
        let Some(buf) = n.buf.clone() else {
            networks.push(NetworkGroup {
                id: n.conn.clone(),
                name: n.name,
                state: n.status.into(),
                status_label: n.status_label,
                is_control: n.is_control,
                collapsed: n.collapsed,
                server_buffer: BufId::server(&n.conn),
                buffers: Vec::new(),
            });
            continue;
        };
        let Some(group) = networks.last_mut() else { continue };
        let picture = match (n.kind, app.conn(&n.conn)) {
            (BufferKind::Query, Some(c)) => nick_pic(h, c, &n.name),
            _ => Pic(None),
        };
        group.buffers.push(BufferRow {
            id: buf,
            kind: n.kind,
            name: n.name,
            unread: n.unread,
            highlights: n.highlights,
            level: n.level,
            joined: n.joined,
            online: n.online,
            typing: n.typing,
            picture,
        });
    }

    // The account shown at the bottom: the open network's, else the first real one.
    let conn = app.active_conn().filter(|c| !c.is_control()).or_else(|| app.conns().find(|c| !c.is_control()));
    let footer = conn.map(|c| FooterView {
        nick: if c.nick.is_empty() { "\u{2014}".into() } else { c.nick.clone() },
        status_text: format!("{} \u{b7} {}", presence(c), c.name()),
        state: c.status.into(),
        away: c.away,
    });
    SidebarView { networks, active: app.active().cloned(), footer, mod_label: if cfg!(target_os = "macos") { "\u{2318}".into() } else { "Ctrl ".into() } }
}

pub fn header(h: &Hearth) -> Option<HeaderView> {
    let app = &h.app;
    let b = app.active_buffer()?;
    let c = app.conn(&b.conn_id)?;
    let id = b.id();
    let subtitle = match b.kind {
        BufferKind::Server if c.is_control() => {
            let n = c.networks.len();
            Some(format!("Bouncer \u{b7} {n} network{}", if n == 1 { "" } else { "s" }))
        }
        BufferKind::Server if c.netid.is_some() => {
            let attrs = c.parent_id.as_ref().and_then(|p| app.conn(p)).zip(c.netid.as_ref()).and_then(|(p, n)| p.networks.get(n));
            Some(match attrs.and_then(|a| a.get("host").filter(|h| !h.is_empty())) {
                Some(host) => match attrs.and_then(|a| a.get("port")).filter(|p| !p.is_empty()) {
                    Some(port) => format!("{host}:{port} via bouncer"),
                    None => format!("{host} via bouncer"),
                },
                None => "via bouncer".into(),
            })
        }
        BufferKind::Server => {
            app.servers.iter().find(|s| s.id == c.server_id).map(|s| format!("{}:{}{}", s.host, s.port, if s.tls { " \u{b7} TLS" } else { "" }))
        }
        BufferKind::Query => b.member(&b.name).and_then(|m| m.user.as_ref().map(|u| format!("{u}@{}", m.host.clone().unwrap_or_default()))),
        BufferKind::Channel => None,
    };
    let topic = (b.kind == BufferKind::Channel && !b.topic.is_empty()).then(|| b.topic.clone());
    let topic_note = topic.as_ref().filter(|_| !b.topic_by.is_empty()).map(|_| {
        let when = if b.topic_time > 0 { format!(" \u{b7} {}", format_full(b.topic_time, app.zone)) } else { String::new() };
        format!("Set by {}{when}", b.topic_by)
    });
    let level = app.notify_level(b);
    Some(HeaderView {
        id,
        kind: b.kind,
        title: if b.kind == BufferKind::Server { c.name().to_string() } else { b.name.clone() },
        topic,
        topic_note,
        subtitle: subtitle.filter(|s| !s.is_empty()),
        joined: b.joined,
        level,
        online: (b.kind == BufferKind::Query).then(|| c.online.get(&c.fold(&b.name)).copied()).flatten(),
        member_count: b.member_count(),
        show_members: app.settings.show_members,
        picture: if b.kind == BufferKind::Query { nick_pic(h, c, &b.name) } else { Pic(None) },
    })
}

/// A warning for the strip under the header when the open network is not connected.
pub fn connection_notice(h: &Hearth) -> Option<(String, bool)> {
    use hearth_model::types::ConnStatus;
    let b = h.app.active_buffer()?;
    if b.kind == BufferKind::Server {
        return None;
    }
    let c = h.app.conn(&b.conn_id)?;
    match c.status {
        ConnStatus::Connected => None,
        ConnStatus::Disconnected => Some((format!("Disconnected{}.", c.error.as_ref().map(|e| format!(" \u{2014} {e}")).unwrap_or_default()), false)),
        _ => Some((format!("Connecting to {}\u{2026}", c.name()), true)),
    }
}

/// `filter` narrows the rows to lines containing it; empty shows everything.
pub fn messages(h: &Hearth, filter: &str) -> Option<MessagesView> {
    let app = &h.app;
    let b = app.active_buffer()?;
    let c = app.conn(&b.conn_id)?;
    let id = b.id();
    Some(MessagesView {
        timeline: app.timeline(&id, filter)?,
        lines_rev: b.lines_rev(),
        me: c.nick.clone(),
        channel: b.kind == BufferKind::Channel,
        can_react: b.kind != BufferKind::Server && c.can_tag("draft/react"),
        flash: app.flash_line(),
        unread: app.summary(&id).unread,
        filter: filter.trim().to_lowercase(),
        buffer: id,
    })
}

pub fn composer(h: &Hearth) -> Option<ComposerView> {
    let app = &h.app;
    let b = app.active_buffer()?;
    let c = app.conn(&b.conn_id)?;
    let id = b.id();
    Some(ComposerView {
        nick: c.nick.clone(),
        placeholder: app.composer_placeholder(&id),
        reply_to: b.reply_to.as_ref().map(|l| ReplyRef { nick: l.nick.clone(), text: strip_formatting(&l.text) }),
        draft: b.composer.text.clone(),
        caret: b.composer.caret,
        typing: b.typing.keys().cloned().collect(),
        suggestions: app.composer_suggestions(&id),
        can_upload: app.can_upload(&id),
        uploading: b.composer.uploading,
        buffer: id,
    })
}

/// What the menu on a nick offers, from the open buffer.
pub fn nick_menu(h: &Hearth, nick: &str) -> Option<NickMenuView> {
    let app = &h.app;
    let id = app.active()?.clone();
    let m = app.nick_menu(&id, nick)?;
    let c = app.conn(&id.conn)?;
    let member = m.member.as_ref();
    let picture = avatar_pic(h, m.avatar.clone());
    Some(NickMenuView {
        nick: m.nick,
        display_name: m.display_name,
        user_host: member.and_then(|x| x.user.as_ref().map(|u| format!("{u}@{}", x.host.clone().unwrap_or_default()))),
        prefix: member.map(|x| x.prefix.clone()).unwrap_or_default(),
        online: m.online,
        away: member.is_some_and(|x| x.away),
        ignored: m.ignored,
        account: member.and_then(|x| x.account.clone()),
        realname: member.and_then(|x| x.realname.clone()).filter(|r| !r.is_empty()),
        is_self: m.is_self,
        can_monitor: m.can_watch,
        watched: m.watched,
        can_moderate: m.can_moderate,
        is_op: m.is_op,
        is_voice: m.is_voice,
        show_avatar: h.app.settings.load_avatars && m.avatar.is_some(),
        picture: if c.profile(nick).avatar.is_some() { picture } else { Pic(None) },
    })
}

pub fn settings_data(h: &Hearth) -> SettingsData {
    let app = &h.app;
    let servers = app
        .servers
        .iter()
        .map(|s| {
            let conn = app.conns().find(|c| c.server_id == s.id && c.parent_id.is_none());
            let is_bouncer = conn.is_some_and(|c| c.is_control());
            let networks = match conn.filter(|_| is_bouncer) {
                Some(c) => app
                    .bouncer_networks(&c.id)
                    .into_iter()
                    .map(|(id, attrs)| {
                        let get = |k: &str| attrs.get(k).cloned().unwrap_or_default();
                        let child = app.conns().find(|k| k.parent_id.as_deref() == Some(c.id.as_str()) && k.netid.as_deref() == Some(id.as_str()));
                        BouncerNetwork {
                            name: attrs.get("name").cloned().unwrap_or_else(|| id.clone()),
                            host: get("host"),
                            port: get("port"),
                            tls: attrs.get("tls").map(String::as_str) != Some("0"),
                            nickname: get("nickname"),
                            username: get("username"),
                            state: child.map_or(ConnState::Disconnected, |k| k.status.into()),
                            error: child.and_then(|k| k.error.clone()),
                            id,
                        }
                    })
                    .collect(),
                None => Vec::new(),
            };
            ServerRow { config: s.clone(), status: conn.map(|c| c.status.into()), is_bouncer, networks }
        })
        .collect();
    SettingsData { settings: app.settings.clone(), servers }
}
