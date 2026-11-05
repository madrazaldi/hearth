//! Ignore masks: "nick", "nick!user@host", with * and ? wildcards.

use hearth_core::irc::message::Source;

/// Expand shorthand into a full nick!user@host mask.
pub fn normalize_mask(input: &str) -> String {
    let m = input.trim();
    if m.is_empty() {
        return String::new();
    }
    if m.contains('!') {
        return if m.contains('@') { m.to_string() } else { format!("{m}@*") };
    }
    if m.contains('@') {
        return format!("*!{m}");
    }
    format!("{m}!*@*")
}

/// Glob match with `*` (any run) and `?` (any one character).
fn glob(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) && pattern[p] != '*' {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// Whether `source` (nick, user, host) matches any mask. `fold` case-folds per
/// the server's casemapping.
pub fn is_ignored(masks: &[String], source: &Source, fold: impl Fn(&str) -> String) -> bool {
    if masks.is_empty() || source.nick.is_empty() {
        return false;
    }
    let full: Vec<char> = fold(&format!("{}!{}@{}", source.nick, source.user.as_deref().unwrap_or(""), source.host.as_deref().unwrap_or(""))).chars().collect();
    masks.iter().any(|raw| {
        let mask: Vec<char> = fold(&normalize_mask(raw)).chars().collect();
        glob(&mask, &full)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::irc::casefold;

    fn src(nick: &str, user: Option<&str>, host: Option<&str>) -> Source {
        Source { nick: nick.into(), user: user.map(String::from), host: host.map(String::from) }
    }

    fn masks(m: &[&str]) -> Vec<String> {
        m.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalizes_shorthand() {
        assert_eq!(normalize_mask("nick"), "nick!*@*");
        assert_eq!(normalize_mask("*@host"), "*!*@host");
        assert_eq!(normalize_mask("nick!user"), "nick!user@*");
        assert_eq!(normalize_mask(" a!b@c "), "a!b@c");
    }

    #[test]
    fn matches_with_wildcards_and_case_folding() {
        let fold = |s: &str| casefold(s, "rfc1459");
        assert!(is_ignored(&masks(&["Troll[1]"]), &src("troll{1}", Some("x"), Some("y")), fold));
        assert!(is_ignored(&masks(&["*!*@*.bad.net"]), &src("a", Some("b"), Some("x.bad.net")), fold));
        assert!(is_ignored(&masks(&["a?c"]), &src("abc", None, None), fold));
        assert!(!is_ignored(&masks(&["a?c"]), &src("abbc", None, None), fold));
        assert!(!is_ignored(&masks(&["x.y"]), &src("xzy", None, None), fold));
    }
}
