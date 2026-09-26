//! IRC message parsing and serialization, including IRCv3 message tags.

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub nick: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub tags: BTreeMap<String, String>,
    pub source: Option<Source>,
    pub command: String,
    pub params: Vec<String>,
}

impl Message {
    pub fn new(command: &str, params: &[&str]) -> Self {
        Message {
            tags: BTreeMap::new(),
            source: None,
            command: command.to_string(),
            params: params.iter().map(|p| p.to_string()).collect(),
        }
    }

    pub fn param(&self, i: usize) -> &str {
        self.params.get(i).map(String::as_str).unwrap_or("")
    }

    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }

    pub fn parse(line: &str) -> Option<Message> {
        let mut rest = line.trim_end_matches(['\r', '\n']).trim_start_matches(' ');
        let mut tags = BTreeMap::new();

        if let Some(stripped) = rest.strip_prefix('@') {
            let (raw_tags, after) = stripped.split_once(' ')?;
            for tag in raw_tags.split(';').filter(|t| !t.is_empty()) {
                let (k, v) = tag.split_once('=').unwrap_or((tag, ""));
                tags.insert(k.to_string(), unescape_tag(v));
            }
            rest = after.trim_start_matches(' ');
        }

        let mut source = None;
        if let Some(stripped) = rest.strip_prefix(':') {
            let (raw, after) = stripped.split_once(' ')?;
            source = Some(parse_source(raw));
            rest = after.trim_start_matches(' ');
        }

        let (command, mut rest) = match rest.split_once(' ') {
            Some((c, r)) => (c, r),
            None => (rest, ""),
        };
        if command.is_empty() {
            return None;
        }

        let mut params = Vec::new();
        loop {
            rest = rest.trim_start_matches(' ');
            if rest.is_empty() {
                break;
            }
            if let Some(trailing) = rest.strip_prefix(':') {
                params.push(trailing.to_string());
                break;
            }
            match rest.split_once(' ') {
                Some((p, r)) => {
                    params.push(p.to_string());
                    rest = r;
                }
                None => {
                    params.push(rest.to_string());
                    break;
                }
            }
        }

        Some(Message {
            tags,
            source,
            command: command.to_ascii_uppercase(),
            params,
        })
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        if !self.tags.is_empty() {
            out.push('@');
            for (i, (k, v)) in self.tags.iter().enumerate() {
                if i > 0 {
                    out.push(';');
                }
                out.push_str(k);
                if !v.is_empty() {
                    out.push('=');
                    out.push_str(&escape_tag(v));
                }
            }
            out.push(' ');
        }
        if let Some(src) = &self.source {
            out.push(':');
            out.push_str(&src.nick);
            if let Some(u) = &src.user {
                out.push('!');
                out.push_str(u);
            }
            if let Some(h) = &src.host {
                out.push('@');
                out.push_str(h);
            }
            out.push(' ');
        }
        out.push_str(&self.command);
        let n = self.params.len();
        for (i, p) in self.params.iter().enumerate() {
            out.push(' ');
            let last = i + 1 == n;
            if last && (p.is_empty() || p.contains(' ') || p.starts_with(':')) {
                out.push(':');
            }
            out.push_str(p);
        }
        out
    }
}

fn parse_source(raw: &str) -> Source {
    let (rest, host) = match raw.split_once('@') {
        Some((r, h)) => (r, Some(h.to_string())),
        None => (raw, None),
    };
    let (nick, user) = match rest.split_once('!') {
        Some((n, u)) => (n.to_string(), Some(u.to_string())),
        None => (rest.to_string(), None),
    };
    Source { nick, user, host }
}

fn unescape_tag(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn escape_tag(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            ';' => out.push_str("\\:"),
            ' ' => out.push_str("\\s"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Case-fold a name according to the server's CASEMAPPING.
pub fn casefold(s: &str, mapping: &str) -> String {
    s.chars()
        .map(|c| match (c, mapping) {
            (_, _) if c.is_ascii_uppercase() => c.to_ascii_lowercase(),
            ('[', "rfc1459" | "rfc1459-strict") => '{',
            (']', "rfc1459" | "rfc1459-strict") => '}',
            ('\\', "rfc1459" | "rfc1459-strict") => '|',
            ('~', "rfc1459") => '^',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_message() {
        let m = Message::parse(
            "@time=2026-01-01T00:00:00.000Z;msgid=a\\sb :nick!user@host PRIVMSG #chan :hello there",
        )
        .unwrap();
        assert_eq!(m.tag("time"), Some("2026-01-01T00:00:00.000Z"));
        assert_eq!(m.tag("msgid"), Some("a b"));
        let src = m.source.as_ref().unwrap();
        assert_eq!(src.nick, "nick");
        assert_eq!(src.user.as_deref(), Some("user"));
        assert_eq!(src.host.as_deref(), Some("host"));
        assert_eq!(m.command, "PRIVMSG");
        assert_eq!(m.params, vec!["#chan", "hello there"]);
    }

    #[test]
    fn parses_bare_and_trailing_edge_cases() {
        let m = Message::parse("PING").unwrap();
        assert!(m.params.is_empty());
        let m = Message::parse(":srv 001 me :").unwrap();
        assert_eq!(m.params, vec!["me", ""]);
        let m = Message::parse("CAP * LS :a b=c").unwrap();
        assert_eq!(m.params, vec!["*", "LS", "a b=c"]);
        let m = Message::parse("@+typing=active TAGMSG #x").unwrap();
        assert_eq!(m.tag("+typing"), Some("active"));
        assert!(Message::parse("").is_none());
    }

    #[test]
    fn roundtrips() {
        let mut m = Message::new("PRIVMSG", &["#a", ":)"]);
        m.tags.insert("+draft/reply".into(), "x;y z".into());
        let s = m.serialize();
        assert_eq!(s, "@+draft/reply=x\\:y\\sz PRIVMSG #a ::)");
        assert_eq!(Message::parse(&s).unwrap(), m);
    }

    #[test]
    fn casefolds() {
        assert_eq!(casefold("Foo[]\\~", "rfc1459"), "foo{}|^");
        assert_eq!(casefold("Foo[]", "ascii"), "foo[]");
    }
}
