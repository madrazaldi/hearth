//! Protocol helpers that don't depend on app state.

use chrono::{DateTime, TimeZone, Utc};
use std::collections::BTreeMap;

/// A server's CASEMAPPING, which decides how nicks and channels compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fold {
    Ascii,
    #[default]
    Rfc1459,
    Rfc1459Strict,
}

impl Fold {
    pub fn from_isupport(value: Option<&str>) -> Fold {
        match value {
            None => Fold::Rfc1459,
            Some("rfc1459") => Fold::Rfc1459,
            Some("rfc1459-strict") => Fold::Rfc1459Strict,
            // "ascii" and anything unknown fold plain letters only.
            Some(_) => Fold::Ascii,
        }
    }

    pub fn apply(self, s: &str) -> String {
        casefold(
            s,
            match self {
                Fold::Ascii => "ascii",
                Fold::Rfc1459 => "rfc1459",
                Fold::Rfc1459Strict => "rfc1459-strict",
            },
        )
    }
}

/// Lowercase `s` and apply the extra folding `mapping` calls for.
pub fn casefold(s: &str, mapping: &str) -> String {
    let strict = mapping == "rfc1459" || mapping == "rfc1459-strict";
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            '[' if strict => '{',
            ']' if strict => '}',
            '\\' if strict => '|',
            '~' if mapping == "rfc1459" => '^',
            c => c,
        })
        .collect()
}

/// The channel membership prefixes a server advertises, in rank order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixMap {
    /// e.g. "qaohv"
    pub modes: String,
    /// e.g. "~&@%+"
    pub symbols: String,
}

impl Default for PrefixMap {
    fn default() -> Self {
        PrefixMap { modes: "ov".into(), symbols: "@+".into() }
    }
}

impl PrefixMap {
    /// Rank of a prefix symbol: lower is more powerful.
    pub fn rank(&self, symbol: char) -> Option<usize> {
        self.symbols.chars().position(|c| c == symbol)
    }

    pub fn has_symbol(&self, c: char) -> bool {
        self.symbols.contains(c)
    }

    pub fn mode_of(&self, symbol: char) -> Option<char> {
        self.rank(symbol).and_then(|i| self.modes.chars().nth(i))
    }

    pub fn symbol_of(&self, mode: char) -> Option<char> {
        self.modes.chars().position(|c| c == mode).and_then(|i| self.symbols.chars().nth(i))
    }
}

pub fn parse_prefix(isupport: &BTreeMap<String, String>) -> PrefixMap {
    let raw = isupport.get("PREFIX").map(String::as_str).unwrap_or("(ov)@+");
    let parsed = raw.strip_prefix('(').and_then(|r| r.split_once(')'));
    match parsed {
        Some((modes, symbols)) if modes.chars().count() == symbols.chars().count() => PrefixMap { modes: modes.into(), symbols: symbols.into() },
        _ => PrefixMap::default(),
    }
}

/// A CTCP payload wrapped in `\x01`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctcp {
    pub command: String,
    pub args: String,
}

pub fn parse_ctcp(text: &str) -> Option<Ctcp> {
    let body = text.strip_prefix('\x01')?;
    let body = body.strip_suffix('\x01').unwrap_or(body);
    Some(match body.split_once(' ') {
        None => Ctcp { command: body.to_uppercase(), args: String::new() },
        Some((c, a)) => Ctcp { command: c.to_uppercase(), args: a.to_string() },
    })
}

pub fn iso_time(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default()
}

/// Parse an ISO-8601 timestamp to epoch milliseconds.
pub fn parse_time(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

/// The `time` tag as milliseconds, else `now`.
pub fn msg_time(tags: &BTreeMap<String, String>, now: i64) -> i64 {
    tags.get("time").and_then(|t| parse_time(t)).unwrap_or(now)
}

fn byte_len(chars: &[char]) -> usize {
    chars.iter().map(|c| c.len_utf8()).sum()
}

/// The most characters of `chars` that fit in `max` bytes.
fn fit_chars(chars: &[char], max: usize) -> usize {
    let mut used = 0;
    for (i, c) in chars.iter().enumerate() {
        used += c.len_utf8();
        if used > max {
            return i;
        }
    }
    chars.len()
}

/// Split text into chunks that fit an IRC line, preferring word boundaries.
pub fn split_message(text: &str, max_bytes: usize) -> Vec<String> {
    if text.len() <= max_bytes {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut rest: Vec<char> = text.chars().collect();
    while byte_len(&rest) > max_bytes {
        let mut cut = fit_chars(&rest, max_bytes).max(1);
        // The last space at or before the cut, if it isn't too far back.
        let space = rest[..(cut + 1).min(rest.len())].iter().rposition(|c| *c == ' ');
        if let Some(s) = space {
            if s as f64 > cut as f64 * 0.6 {
                cut = s;
            }
        }
        out.push(rest[..cut].iter().collect());
        let mut next = cut;
        if rest.get(next) == Some(&' ') {
            next += 1;
        }
        rest = rest[next..].to_vec();
    }
    if !rest.is_empty() {
        out.push(rest.iter().collect());
    }
    out
}

/// Split without losing anything, since the pieces are concatenated as-is:
/// cuts go after a space when there's one near the end.
pub fn split_exact(text: &str, max_bytes: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<char> = text.chars().collect();
    while byte_len(&rest) > max_bytes {
        let mut cut = fit_chars(&rest, max_bytes).max(1);
        if let Some(space) = rest[..cut].iter().rposition(|c| *c == ' ') {
            if (space + 1) as f64 > cut as f64 * 0.6 {
                cut = space + 1;
            }
        }
        out.push(rest[..cut].iter().collect());
        rest = rest[cut..].to_vec();
    }
    out.push(rest.iter().collect());
    out
}

/// Parse "a=b;c=d" attribute lists used by soju BOUNCER messages.
pub fn parse_attrs(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if s.is_empty() || s == "*" {
        return out;
    }
    for part in s.split(';') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let mut value = String::new();
        let mut chars = v.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                value.push(c);
                continue;
            }
            match chars.next() {
                Some(':') => value.push(';'),
                Some('s') => value.push(' '),
                Some('\\') => value.push('\\'),
                Some('r') => value.push('\r'),
                Some('n') => value.push('\n'),
                Some(other) => value.push(other),
                None => {}
            }
        }
        out.insert(k.to_string(), value);
    }
    out
}

pub fn escape_attr(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\:").replace(' ', "\\s")
}

/// Escape an IRCv3 message-tag value.
pub fn escape_tag(v: &str) -> String {
    v.replace('\\', "\\\\").replace(';', "\\:").replace(' ', "\\s").replace('\r', "\\r").replace('\n', "\\n")
}

/// Split "nick!user@host" into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    pub nick: String,
    pub user: Option<String>,
    pub host: Option<String>,
}

pub fn parse_mask(s: &str) -> Mask {
    let (nick_user, host) = match s.split_once('@') {
        Some((a, h)) => (a, Some(h.to_string())),
        None => (s, None),
    };
    let (nick, user) = match nick_user.split_once('!') {
        Some((n, u)) => (n.to_string(), Some(u.to_string())),
        None => (nick_user.to_string(), None),
    };
    Mask { nick, user, host }
}

/// Whether `name` starts with one of the server's channel prefixes.
pub fn is_channel_name(name: &str, chantypes: &str) -> bool {
    name.chars().next().is_some_and(|c| chantypes.contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_word_boundaries_within_the_byte_budget() {
        let text = "lorem ipsum dolor sit amet ".repeat(20);
        let text = text.trim();
        let parts = split_message(text, 100);
        assert!(parts.iter().all(|p| p.len() <= 100));
        assert_eq!(parts.join(" "), text);
    }

    #[test]
    fn never_splits_inside_a_multi_byte_character() {
        let text = "é".repeat(100);
        let parts = split_message(&text, 51);
        assert_eq!(parts.concat(), text);
        assert!(parts.iter().all(|p| p.len() <= 51));
        assert_eq!(split_exact(&text, 51).concat(), text);
    }

    #[test]
    fn casefolds_per_mapping() {
        assert_eq!(casefold("Troll[1]~\\", "rfc1459"), "troll{1}^|");
        assert_eq!(casefold("Troll[1]~\\", "rfc1459-strict"), "troll{1}~|");
        assert_eq!(casefold("Troll[1]", "ascii"), "troll[1]");
    }

    #[test]
    fn parses_prefix_and_ctcp() {
        let mut is = BTreeMap::new();
        is.insert("PREFIX".to_string(), "(qaohv)~&@%+".to_string());
        let p = parse_prefix(&is);
        assert_eq!((p.modes.as_str(), p.symbols.as_str()), ("qaohv", "~&@%+"));
        assert_eq!(p.mode_of('@'), Some('o'));
        is.insert("PREFIX".to_string(), "(ov)@".to_string());
        assert_eq!(parse_prefix(&is), PrefixMap::default());
        assert_eq!(parse_ctcp("\x01ACTION waves\x01"), Some(Ctcp { command: "ACTION".into(), args: "waves".into() }));
        assert_eq!(parse_ctcp("\x01version"), Some(Ctcp { command: "VERSION".into(), args: String::new() }));
        assert_eq!(parse_ctcp("plain"), None);
    }

    #[test]
    fn parses_attrs_and_times() {
        let a = parse_attrs("name=My\\sNet;state=connected;flag");
        assert_eq!(a["name"], "My Net");
        assert_eq!(a["flag"], "");
        assert_eq!(escape_attr("a b;c\\"), "a\\sb\\:c\\\\");
        assert_eq!(iso_time(1767225600123), "2026-01-01T00:00:00.123Z");
        assert_eq!(parse_time("2026-01-01T00:00:00.123Z"), Some(1767225600123));
        assert_eq!(parse_mask("n!u@h"), Mask { nick: "n".into(), user: Some("u".into()), host: Some("h".into()) });
    }
}
