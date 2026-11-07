//! Emoji shortcodes from gemoji (`scripts/gen-emoji.sh` regenerates `emoji.txt`), for `:name` suggestions and `:name:` expansion.

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

const DATA: &str = include_str!("emoji.txt");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emoji {
    pub name: &'static str,
    pub ch: &'static str,
}

fn all() -> &'static [Emoji] {
    static TABLE: OnceLock<Vec<Emoji>> = OnceLock::new();
    TABLE.get_or_init(|| DATA.lines().filter_map(|l| l.split_once(' ')).map(|(name, ch)| Emoji { name, ch }).collect())
}

pub fn emoji_by_name(name: &str) -> Option<&'static str> {
    static BY_NAME: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    BY_NAME.get_or_init(|| all().iter().map(|e| (e.name, e.ch)).collect()).get(name.to_lowercase().as_str()).copied()
}

// The colon has to start a word, so `12:30`, URLs and `nick:` don't trigger it.
fn query_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)(:([A-Za-z0-9_+-]+))$").unwrap())
}

fn shortcode_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)(:([A-Za-z0-9_+-]+):)$").unwrap())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiQuery {
    /// Byte offset of the colon.
    pub start: usize,
    pub query: String,
}

/// The `:name` being typed at the end of `before`, with where its colon is.
pub fn emoji_query(before: &str) -> Option<EmojiQuery> {
    let m = query_re().captures(before)?;
    Some(EmojiQuery { start: before.len() - m[1].len(), query: m[2].to_lowercase() })
}

/// Names starting with the query first, then names containing it, each in gemoji order.
pub fn search_emoji(query: &str, limit: usize) -> Vec<Emoji> {
    let q = query.to_lowercase();
    let mut out: Vec<Emoji> = all().iter().filter(|e| e.name.starts_with(&q)).copied().collect();
    if out.len() < limit {
        out.extend(all().iter().filter(|e| !e.name.starts_with(&q) && e.name.contains(&q)).copied());
    }
    out.truncate(limit);
    out
}

/// A complete `:name:` just typed at the end of `before`, as its emoji and where it starts.
pub fn typed_shortcode(before: &str) -> Option<(usize, &'static str)> {
    let m = shortcode_re().captures(before)?;
    let ch = emoji_by_name(&m[2])?;
    Some((before.len() - m[1].len(), ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_query_being_typed() {
        assert_eq!(emoji_query(":j"), Some(EmojiQuery { start: 0, query: "j".into() }));
        assert_eq!(emoji_query("lol :Jo"), Some(EmojiQuery { start: 4, query: "jo".into() }));
        assert_eq!(emoji_query("nice :+1"), Some(EmojiQuery { start: 5, query: "+1".into() }));
    }

    #[test]
    fn ignores_colons_that_do_not_start_a_word() {
        for s in ["at 12:30", "https://x", "nick:", ":joy: ", ":"] {
            assert_eq!(emoji_query(s), None, "{s}");
        }
    }

    #[test]
    fn ranks_prefix_matches_first_in_gemoji_order() {
        let names: Vec<_> = search_emoji("jo", 7).iter().map(|e| e.name).collect();
        assert_eq!(names[0], "joy");
        assert!(search_emoji("j", 50).iter().any(|e| e.name == "joy"));
        assert_eq!(search_emoji("j", 7).len(), 7);
    }

    #[test]
    fn falls_back_to_names_containing_the_query() {
        let names: Vec<_> = search_emoji("eyes", 7).iter().map(|e| e.name).collect();
        assert_eq!(names[0], "eyes");
        assert!(names.contains(&"heart_eyes"));
        assert!(search_emoji("zzzqqq", 7).is_empty());
    }

    #[test]
    fn expands_a_complete_shortcode() {
        assert_eq!(emoji_by_name("joy"), Some("😂"));
        assert_eq!(typed_shortcode("haha :joy:"), Some((5, "😂")));
        assert_eq!(typed_shortcode(":THUMBSUP:"), Some((0, "👍")));
        assert_eq!(typed_shortcode("haha :notanemoji:"), None);
        assert_eq!(typed_shortcode("a:joy:"), None);
    }
}
