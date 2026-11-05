//! What each kind of row says: event sentences, collapsed-run summaries,
//! whois cards and per-message extras (display name, link to preview).

use crate::buffer::{BufId, BufferKind};
use crate::format::{first_link, strip_formatting};
use crate::media::is_nsfw;
use crate::state::App;
use crate::time::{format_clock_for, format_duration, format_full};
use crate::types::{Line, LineType};

/// The mark that stands in for the sender on an event row.
pub fn event_glyph(kind: LineType) -> &'static str {
    match kind {
        LineType::Join => "\u{2192}",
        LineType::Part | LineType::Quit => "\u{2190}",
        LineType::Nick => "\u{21bb}",
        LineType::Kick => "\u{2a2f}",
        LineType::Mode => "\u{b1}",
        LineType::Topic => "\u{b6}",
        LineType::Invite => "\u{2709}",
        LineType::Error => "!",
        _ => "\u{b7}",
    }
}

/// One styled piece of an event sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPart {
    /// A nick the user can click.
    Nick(String),
    Text(String),
    /// De-emphasised (a host, a reason).
    Faint(String),
    Mono(String),
    /// Text with IRC formatting, set apart (a topic).
    Quote(String),
    /// Text with IRC formatting, as is.
    Rich(String),
    /// A "Join" button for the channel.
    JoinButton(String),
}

/// The sentence for an event row (join, part, kick, mode, ...).
pub fn event_parts(l: &Line) -> Vec<EventPart> {
    use EventPart::*;
    let reason = |l: &Line| if l.text.is_empty() { None } else { Some(Faint(format!(" ({})", strip_formatting(&l.text)))) };
    let mut v = Vec::new();
    match l.kind {
        LineType::Join => {
            v.extend([Nick(l.nick.clone()), Text(" joined ".into())]);
            if let Some(h) = l.data.as_ref().and_then(|d| d.get("host")).filter(|h| !h.is_empty() && *h != "@") {
                v.push(Faint(h.clone()));
            }
        }
        LineType::Part => {
            v.extend([Nick(l.nick.clone()), Text(" left".into())]);
            v.extend(reason(l));
        }
        LineType::Quit => {
            v.extend([Nick(l.nick.clone()), Text(" quit".into())]);
            v.extend(reason(l));
        }
        LineType::Nick => v.extend([Nick(l.nick.clone()), Text(" is now ".into()), Nick(l.target.clone().unwrap_or_default())]),
        LineType::Kick => {
            v.extend([Nick(l.nick.clone()), Text(" removed ".into()), Nick(l.target.clone().unwrap_or_default())]);
            v.extend(reason(l));
        }
        LineType::Mode => {
            if l.nick.is_empty() {
                v.push(Text("Mode ".into()));
            } else {
                v.extend([Nick(l.nick.clone()), Text(" set mode ".into())]);
            }
            v.push(Mono(l.text.clone()));
        }
        LineType::Topic => v.extend([Nick(l.nick.clone()), Text(" changed the topic to ".into()), Quote(l.text.clone())]),
        LineType::Invite => {
            v.extend([Nick(l.nick.clone()), Text(format!(" {}", l.text))]);
            if let Some(t) = &l.target {
                v.push(JoinButton(t.clone()));
            }
        }
        _ => v.push(Rich(l.text.clone())),
    }
    v
}

/// "ann, bob joined · cy left · 2 nick changes" for a collapsed run.
pub fn summarize_run(lines: &[&Line]) -> String {
    let (mut joined, mut left, mut renamed): (Vec<&str>, Vec<&str>, Vec<String>) = (Vec::new(), Vec::new(), Vec::new());
    for l in lines {
        match l.kind {
            LineType::Join if !joined.contains(&l.nick.as_str()) => joined.push(&l.nick),
            LineType::Part | LineType::Quit if !left.contains(&l.nick.as_str()) => left.push(&l.nick),
            LineType::Nick => renamed.push(format!("{} \u{2192} {}", l.nick, l.target.as_deref().unwrap_or(""))),
            _ => {}
        }
    }
    let names = |a: &[&str]| if a.len() <= 3 { a.join(", ") } else { format!("{} and {} others", a[..2].join(", "), a.len() - 2) };
    let mut parts: Vec<String> = Vec::new();
    if !joined.is_empty() {
        parts.push(format!("{} joined", names(&joined)));
    }
    if !left.is_empty() {
        parts.push(format!("{} left", names(&left)));
    }
    if !renamed.is_empty() {
        parts.push(if renamed.len() <= 2 { renamed.join(", ") } else { format!("{} nick changes", renamed.len()) });
    }
    parts.join(" \u{b7} ")
}

/// How a line is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// privmsg, notice or action.
    Talk,
    /// A one-line event.
    Event,
    /// A whois result card.
    Whois,
    /// A foldable block of text (MOTD, /help).
    Block,
}

pub fn row_kind(l: &Line) -> RowKind {
    match l.kind {
        LineType::Privmsg | LineType::Notice | LineType::Action => RowKind::Talk,
        LineType::Whois => RowKind::Whois,
        _ if l.data.as_ref().is_some_and(|d| d.contains_key("motd")) => RowKind::Block,
        _ => RowKind::Event,
    }
}

/// A MOTD-style block: a title and its text.
pub fn block_of(l: &Line) -> Option<(String, String)> {
    let d = l.data.as_ref()?;
    Some((d.get("title").cloned().unwrap_or_else(|| "Message of the day".into()), d.get("motd")?.clone()))
}

/// A whois result, ready to lay out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoisCard {
    pub nick: String,
    pub user_host: String,
    /// Label and value, in display order; empty values are left out.
    pub rows: Vec<WhoisRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoisRow {
    pub label: &'static str,
    pub value: String,
    pub mono: bool,
    /// Rich text (the real name can carry formatting codes).
    pub rich: bool,
}

impl App {
    /// The card for a whois line.
    pub fn whois_card(&self, l: &Line) -> Option<WhoisCard> {
        let w = l.data.as_ref().filter(|_| l.kind == LineType::Whois)?;
        let get = |k: &str| w.get(k).filter(|v| !v.is_empty());
        let put = |label, key: &str, mono: bool, rich: bool| get(key).map(|v| WhoisRow { label, value: v.clone(), mono, rich });
        let mut rows: Vec<WhoisRow> = Vec::new();
        rows.extend(put("Name", "realname", false, true));
        rows.extend(put("Account", "account", false, false));
        rows.extend(put("Server", "server", false, false));
        rows.extend(put("Channels", "channels", true, false));
        if let Some(idle) = get("idle") {
            let mut v = format_duration(idle.parse().unwrap_or(0));
            if let Some(on) = get("signon").and_then(|s| s.parse::<i64>().ok()) {
                v += &format!(" \u{b7} online since {}", format_full(on * 1000, self.zone));
            }
            rows.push(WhoisRow { label: "Idle", value: v, mono: false, rich: false });
        }
        rows.extend(put("Away", "away", false, false));
        rows.extend(put("Operator", "oper", false, false));
        rows.extend(put("Security", "secure", false, false));
        rows.extend(put("Actual", "actual", true, false));
        Some(WhoisCard {
            nick: get("nick").cloned().unwrap_or_default(),
            user_host: format!("{}@{}", w.get("user").map(String::as_str).unwrap_or(""), w.get("host").map(String::as_str).unwrap_or("")),
            rows,
        })
    }

    /// The clock text of a row (`3:04p`, `15:04`) per the settings.
    pub fn clock(&self, ms: i64) -> String {
        format_clock_for(ms, &self.settings, self.zone)
    }

    /// The long form shown as a tooltip on the clock.
    pub fn clock_full(&self, ms: i64) -> String {
        format_full(ms, self.zone)
    }

    /// Per-message extras the row needs beyond the line itself.
    pub fn row_info(&self, id: &BufId, l: &Line) -> RowInfo {
        let Some(c) = self.conns.get(&id.conn) else { return RowInfo::default() };
        let b = c.buf(&id.key);
        let parent = l.reply_to.as_ref().filter(|_| !l.redacted).and_then(|m| b?.find_msgid(m)).map(|p| (p.nick.clone(), strip_formatting(&p.text)));
        let kind = b.map_or(BufferKind::Server, |b| b.kind);
        RowInfo {
            clock: self.clock(l.time),
            full_time: self.clock_full(l.time),
            display_name: c.profile(&l.nick).display_name,
            avatar: c.profile(&l.nick).avatar,
            preview_url: (self.settings.link_previews && !l.redacted && kind != BufferKind::Server).then(|| first_link(&l.text)).flatten(),
            nsfw: is_nsfw(&l.text),
            reply_to: parent,
        }
    }

    /// Whether the message list should keep the user at the bottom: new
    /// lines while pinned are read and, with the window focused, marked so.
    pub fn viewport_changed(&mut self, id: &BufId, at_bottom: bool, near_top: bool, filtering: bool) {
        self.at_bottom = at_bottom;
        if filtering {
            return;
        }
        if near_top {
            self.fetch_history(id, crate::history::HistMode::Before);
        }
        if at_bottom && self.focused {
            self.mark_read(id);
        }
    }
}

/// Extras for one message row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowInfo {
    pub clock: String,
    pub full_time: String,
    pub display_name: Option<String>,
    /// A metadata avatar URL: ask `App::request_avatar` for it.
    pub avatar: Option<String>,
    /// A link to preview: ask `App::request_preview` for it.
    pub preview_url: Option<String>,
    pub nsfw: bool,
    /// Nick and plain text of the message this one replies to.
    pub reply_to: Option<(String, String)>,
}

/// A network tile's letter, or an avatar fallback: the first letter or digit, upper-cased.
pub fn initial(s: &str) -> String {
    s.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: LineType, nick: &str) -> Line {
        Line::new(1, 0, kind, nick, "")
    }

    #[test]
    fn summarizes_runs() {
        let lines: Vec<Line> =
            vec![ev(LineType::Join, "a"), ev(LineType::Join, "b"), ev(LineType::Join, "c"), ev(LineType::Join, "d"), ev(LineType::Part, "e")];
        let refs: Vec<&Line> = lines.iter().collect();
        assert_eq!(summarize_run(&refs), "a, b and 2 others joined \u{b7} e left");
        let mut n = ev(LineType::Nick, "x");
        n.target = Some("y".into());
        assert_eq!(summarize_run(&[&n]), "x \u{2192} y");
    }

    #[test]
    fn words_events() {
        let mut k = ev(LineType::Kick, "op");
        k.target = Some("troll".into());
        k.text = "bye".into();
        assert_eq!(
            event_parts(&k),
            vec![EventPart::Nick("op".into()), EventPart::Text(" removed ".into()), EventPart::Nick("troll".into()), EventPart::Faint(" (bye)".into())]
        );
        assert_eq!(initial("#chan"), "C");
        assert_eq!(initial("???"), "?");
    }
}
