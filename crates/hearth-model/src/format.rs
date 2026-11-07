//! mIRC formatting codes and link detection → styled spans, plus the
//! highlight-rule matcher (which works on text with the codes stripped).

use regex::{Regex, RegexBuilder};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A colour from a formatting code. The 16 basic colours depend on the theme,
/// so they stay symbolic; resolve them with [`irc_palette`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// mIRC colours 0–15, drawn from the theme's palette.
    Palette(u8),
    /// A fixed 0xRRGGBB colour (extended 16–98 table or a hex code).
    Rgb(u32),
    /// The theme's text colour (produced by reverse video).
    Foreground,
    /// The theme's background colour (produced by reverse video).
    Background,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub mono: bool,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    /// Set for a link; `www.` links already carry their `https://`.
    pub href: Option<String>,
    /// Set for a `#channel` the user can jump to.
    pub channel: Option<String>,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Span {
        Span { text: text.into(), ..Span::default() }
    }
}

/// The 16 basic mIRC colours as 0xRRGGBB, matching the theme palettes in app.css.
pub fn irc_palette(dark: bool) -> [u32; 16] {
    if dark {
        [
            0xf2f2f2, 0x6b6b6b, 0x6f8fd6, 0x7fb069, 0xe5705f, 0xb26a52, 0xb58ad8, 0xe39b4c, 0xe3cf62, 0x98d67f, 0x5fb3b3, 0x78d2d9, 0x82a6f2, 0xe188c6,
            0x8c8c8c, 0xbdbdbd,
        ]
    } else {
        [
            0xffffff, 0x1e1e1e, 0x2f4fa8, 0x3f7a2c, 0xc2412f, 0x7c3b24, 0x7a3fa5, 0xc06a14, 0xa58a0a, 0x4b9a2d, 0x1f7f7f, 0x1c8e99, 0x3864d0, 0xb0398d,
            0x6f6f6f, 0xa8a8a8,
        ]
    }
}

/// Colours 16–98 from the extended mIRC table.
const EXTENDED: [u32; 83] = [
    0x470000, 0x472100, 0x474700, 0x324700, 0x004700, 0x00472c, 0x004747, 0x002747, 0x000047, 0x2e0047, 0x470047, 0x47002a, 0x740000, 0x743a00, 0x747400,
    0x517400, 0x007400, 0x007449, 0x007474, 0x004074, 0x000074, 0x4b0074, 0x740074, 0x740045, 0xb50000, 0xb56300, 0xb5b500, 0x7db500, 0x00b500, 0x00b571,
    0x00b5b5, 0x0063b5, 0x0000b5, 0x7500b5, 0xb500b5, 0xb5006b, 0xff0000, 0xff8c00, 0xffff00, 0xb2ff00, 0x00ff00, 0x00ffa0, 0x00ffff, 0x008cff, 0x0000ff,
    0xa500ff, 0xff00ff, 0xff0098, 0xff5959, 0xffb459, 0xffff71, 0xcfff60, 0x6fff6f, 0x65ffc9, 0x6dffff, 0x59b4ff, 0x5959ff, 0xc459ff, 0xff66ff, 0xff59bc,
    0xff9c9c, 0xffd39c, 0xffff9c, 0xe2ff9c, 0x9cff9c, 0x9cffdb, 0x9cffff, 0x9cd3ff, 0x9c9cff, 0xdc9cff, 0xff9cff, 0xff94d3, 0x000000, 0x131313, 0x282828,
    0x363636, 0x4d4d4d, 0x656565, 0x818181, 0x9f9f9f, 0xbcbcbc, 0xe2e2e2, 0xffffff,
];

fn color_of(n: u32) -> Option<Color> {
    match n {
        0..=15 => Some(Color::Palette(n as u8)),
        16..=98 => Some(Color::Rgb(EXTENDED[(n - 16) as usize])),
        _ => None,
    }
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| RegexBuilder::new(r#"\b(?:https?://|www\.)[^\s<>"'\x00-\x1f]+|\bmailto:[^\s<>"']+"#).case_insensitive(true).build().unwrap())
}

fn chan_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(^|[\s(])(#[^\s,\x07()]{1,64})").unwrap())
}

fn strip_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x03(?:[0-9]{1,2}(?:,[0-9]{1,2})?)?|\x04(?:[0-9a-fA-F]{6}(?:,[0-9a-fA-F]{6})?)?|[\x02\x0f\x11\x16\x1d\x1e\x1f]").unwrap())
}

pub fn strip_formatting(s: &str) -> String {
    strip_re().replace_all(s, "").into_owned()
}

/// Up to two ASCII digits from the start of `chars`; the value and how many were read.
fn digits(chars: &[char]) -> Option<(u32, usize)> {
    let n = chars.iter().take(2).take_while(|c| c.is_ascii_digit()).count();
    (n > 0).then(|| (chars[..n].iter().collect::<String>().parse().unwrap(), n))
}

fn hex6(chars: &[char]) -> Option<u32> {
    (chars.len() >= 6 && chars[..6].iter().all(|c| c.is_ascii_hexdigit())).then(|| u32::from_str_radix(&chars[..6].iter().collect::<String>(), 16).unwrap())
}

pub fn parse_formatting(input: &str) -> Vec<Span> {
    let chars: Vec<char> = input.chars().collect();
    let mut out: Vec<Span> = Vec::new();
    let mut cur = Span::default();
    fn push(out: &mut Vec<Span>, cur: &mut Span) {
        if !cur.text.is_empty() {
            out.push(cur.clone());
        }
        cur.text.clear();
    }
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\x02' => {
                push(&mut out, &mut cur);
                cur.bold = !cur.bold;
                i += 1;
            }
            '\x1d' => {
                push(&mut out, &mut cur);
                cur.italic = !cur.italic;
                i += 1;
            }
            '\x1f' => {
                push(&mut out, &mut cur);
                cur.underline = !cur.underline;
                i += 1;
            }
            '\x1e' => {
                push(&mut out, &mut cur);
                cur.strike = !cur.strike;
                i += 1;
            }
            '\x11' => {
                push(&mut out, &mut cur);
                cur.mono = !cur.mono;
                i += 1;
            }
            '\x16' => {
                push(&mut out, &mut cur);
                let (fg, bg) = (cur.fg, cur.bg);
                cur.fg = Some(bg.unwrap_or(Color::Background));
                cur.bg = Some(fg.unwrap_or(Color::Foreground));
                i += 1;
            }
            '\x0f' => {
                push(&mut out, &mut cur);
                cur = Span::default();
                i += 1;
            }
            '\x03' => {
                push(&mut out, &mut cur);
                match digits(&chars[i + 1..]) {
                    None => {
                        cur.fg = None;
                        cur.bg = None;
                        i += 1;
                    }
                    Some((fg, n)) => {
                        cur.fg = color_of(fg);
                        let mut used = 1 + n;
                        if chars.get(i + used) == Some(&',') {
                            if let Some((bg, m)) = digits(&chars[i + used + 1..]) {
                                cur.bg = color_of(bg);
                                used += 1 + m;
                            }
                        }
                        i += used;
                    }
                }
            }
            '\x04' => {
                push(&mut out, &mut cur);
                match hex6(&chars[i + 1..]) {
                    None => {
                        cur.fg = None;
                        cur.bg = None;
                        i += 1;
                    }
                    Some(fg) => {
                        cur.fg = Some(Color::Rgb(fg));
                        let mut used = 7;
                        if chars.get(i + used) == Some(&',') {
                            if let Some(bg) = hex6(&chars[i + used + 1..]) {
                                cur.bg = Some(Color::Rgb(bg));
                                used += 7;
                            }
                        }
                        i += used;
                    }
                }
            }
            c => {
                cur.text.push(c);
                i += 1;
            }
        }
    }
    push(&mut out, &mut cur);
    out.into_iter().flat_map(linkify).collect()
}

fn trim_url(mut url: &str) -> &str {
    // Drop trailing punctuation, but keep a closing paren that balances one in the URL.
    while let Some(last) = url.chars().last() {
        let punct = matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | ']' | '>');
        let unbalanced = last == ')' && url.matches('(').count() < url.matches(')').count();
        if !(punct || unbalanced) {
            break;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
    url
}

fn with_scheme(url: &str) -> String {
    if url.len() >= 4 && url[..4].eq_ignore_ascii_case("www.") {
        format!("https://{url}")
    } else {
        url.to_string()
    }
}

struct Found {
    start: usize,
    end: usize,
    href: Option<String>,
    channel: Option<String>,
}

fn linkify(seg: Span) -> Vec<Span> {
    let text = &seg.text;
    let mut found: Vec<Found> = Vec::new();
    for m in url_re().find_iter(text) {
        let url = trim_url(m.as_str());
        found.push(Found { start: m.start(), end: m.start() + url.len(), href: Some(with_scheme(url)), channel: None });
    }
    for c in chan_re().captures_iter(text) {
        let (lead, name) = (c.get(1).unwrap(), c.get(2).unwrap());
        let name = name.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"']);
        let start = lead.end();
        let end = start + name.len();
        if !found.iter().any(|f| start < f.end && end > f.start) {
            found.push(Found { start, end, href: None, channel: Some(name.to_string()) });
        }
    }
    if found.is_empty() {
        return vec![seg];
    }
    found.sort_by_key(|f| f.start);
    let mut out = Vec::new();
    let mut pos = 0;
    for f in found {
        if f.start > pos {
            out.push(Span { text: text[pos..f.start].to_string(), href: None, channel: None, ..seg.clone() });
        }
        out.push(Span { text: text[f.start..f.end].to_string(), href: f.href, channel: f.channel, ..seg.clone() });
        pos = f.end;
    }
    if pos < text.len() {
        out.push(Span { text: text[pos..].to_string(), href: None, channel: None, ..seg.clone() });
    }
    out
}

// ─── Highlight rules ───────────────────────────────────────────

thread_local! {
    static RULES: RefCell<HashMap<String, Option<Regex>>> = RefCell::new(HashMap::new());
}

/// Splits `/pattern/flags` into its parts.
fn split_rule(rule: &str) -> Option<(&str, &str)> {
    let rest = rule.strip_prefix('/')?;
    let slash = rest.rfind('/')?;
    let (pattern, flags) = (&rest[..slash], &rest[slash + 1..]);
    (!pattern.is_empty() && flags.chars().all(|c| c.is_ascii_lowercase())).then_some((pattern, flags))
}

/// Build a regex from JS-style flags. `g` and `y` only make JS matching
/// stateful, so they are accepted and ignored; unknown or repeated flags are an error.
fn build_regex(pattern: &str, flags: &str) -> Result<Regex, String> {
    let mut seen = String::new();
    let mut b = RegexBuilder::new(pattern);
    for f in flags.chars() {
        if seen.contains(f) || !"dgimsuvy".contains(f) {
            return Err(format!("Invalid flags supplied to RegExp constructor '{flags}'"));
        }
        seen.push(f);
        match f {
            'i' => {
                b.case_insensitive(true);
            }
            'm' => {
                b.multi_line(true);
            }
            's' => {
                b.dot_matches_new_line(true);
            }
            _ => {}
        }
    }
    b.build().map_err(|e| {
        // The last line of regex's message is the human part.
        e.to_string().lines().last().unwrap_or("invalid pattern").trim().trim_start_matches("error: ").to_string()
    })
}

fn compile_rule(rule: &str) -> Option<Regex> {
    RULES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(rule) {
            return hit.clone();
        }
        let re = match split_rule(rule) {
            Some((p, f)) => build_regex(p, f).ok(),
            None => {
                let word = format!("(^|[^A-Za-z0-9_-]){}($|[^A-Za-z0-9_-])", regex::escape(rule));
                RegexBuilder::new(&word).case_insensitive(true).build().ok()
            }
        };
        if cache.len() > 500 {
            cache.clear();
        }
        cache.insert(rule.to_string(), re.clone());
        re
    })
}

/// Why a highlight rule is invalid, or `None`. `/pattern/flags` rules are
/// regexes; anything else is a word.
pub fn rule_error(rule: &str) -> Option<String> {
    let (pattern, flags) = split_rule(rule.trim())?;
    build_regex(pattern, flags).err()
}

/// The first rule that matches: words match whole-word and case-insensitively,
/// `/regex/flags` as written. Invalid rules never match.
pub fn matching_rule<'a>(text: &str, rules: &'a [String]) -> Option<&'a String> {
    let plain = strip_formatting(text);
    rules.iter().find(|r| !r.is_empty() && compile_rule(r).is_some_and(|re| re.is_match(&plain)))
}

pub fn mentions(text: &str, rules: &[String]) -> bool {
    matching_rule(text, rules).is_some()
}

/// The first http(s) link in a message, for previews.
pub fn first_link(text: &str) -> Option<String> {
    let plain = strip_formatting(text);
    for m in url_re().find_iter(&plain) {
        let url = trim_url(m.as_str());
        if url.len() >= 7 && url[..7].eq_ignore_ascii_case("mailto:") {
            continue;
        }
        return Some(with_scheme(url));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "\x02";
    const C: &str = "\x03";
    const HEX: &str = "\x04";
    const I: &str = "\x1d";
    const U: &str = "\x1f";
    const S: &str = "\x1e";
    const M: &str = "\x11";
    const R: &str = "\x16";
    const O: &str = "\x0f";

    /// Compact spans for comparison: text plus the interesting attributes.
    fn seg(s: &str) -> Vec<String> {
        parse_formatting(s)
            .into_iter()
            .map(|x| {
                let mut out = format!("{:?}", x.text);
                for (on, name) in [(x.bold, "bold"), (x.italic, "italic"), (x.underline, "underline"), (x.strike, "strike"), (x.mono, "mono")] {
                    if on {
                        out += &format!(" {name}");
                    }
                }
                if let Some(c) = x.fg {
                    out += &format!(" fg={c:?}");
                }
                if let Some(c) = x.bg {
                    out += &format!(" bg={c:?}");
                }
                if let Some(h) = x.href {
                    out += &format!(" href={h}");
                }
                if let Some(c) = x.channel {
                    out += &format!(" chan={c}");
                }
                out
            })
            .collect()
    }

    fn links(s: &str) -> Vec<String> {
        parse_formatting(s).into_iter().filter_map(|x| x.href).collect()
    }

    fn chans(s: &str) -> Vec<String> {
        parse_formatting(s).into_iter().filter_map(|x| x.channel).collect()
    }

    fn rules(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn toggles_bold_italic_underline_strike_and_monospace() {
        assert_eq!(
            seg(&format!("a{B}b{I}c{B}d{I}{U}e{U}{S}f{S}{M}g")),
            ["\"a\"", "\"b\" bold", "\"c\" bold italic", "\"d\" italic", "\"e\" underline", "\"f\" strike", "\"g\" mono"]
        );
    }

    #[test]
    fn resets_everything_with_ctrl_o() {
        assert_eq!(seg(&format!("{B}{C}4bold red{O}plain")), ["\"bold red\" bold fg=Palette(4)", "\"plain\""]);
    }

    #[test]
    fn parses_basic_colours_with_and_without_backgrounds() {
        assert_eq!(
            seg(&format!("{C}4red{C}04,2red on blue{C}3green keeps bg{C}plain")),
            ["\"red\" fg=Palette(4)", "\"red on blue\" fg=Palette(4) bg=Palette(2)", "\"green keeps bg\" fg=Palette(3) bg=Palette(2)", "\"plain\""]
        );
    }

    #[test]
    fn reads_at_most_two_digits_so_trailing_digits_are_text() {
        assert_eq!(seg(&format!("{C}1234")), ["\"34\" fg=Palette(12)"]);
        assert_eq!(seg(&format!("{C}4,123")), ["\"3\" fg=Palette(4) bg=Palette(12)"]);
    }

    #[test]
    fn keeps_a_comma_that_is_not_followed_by_a_colour() {
        assert_eq!(seg(&format!("{C}4,hello")), ["\",hello\" fg=Palette(4)"]);
        assert_eq!(seg(&format!("{C},4x")), ["\",4x\""]);
    }

    #[test]
    fn maps_the_extended_palette_and_treats_99_as_default() {
        assert_eq!(seg(&format!("{C}16a{C}52b{C}98c{C}99d")), ["\"a\" fg=Rgb(4653056)", "\"b\" fg=Rgb(16711680)", "\"c\" fg=Rgb(16777215)", "\"d\""]);
        assert_eq!(seg(&format!("{C}88,99x")), ["\"x\" fg=Rgb(0)"]);
    }

    #[test]
    fn parses_hex_colours() {
        assert_eq!(
            seg(&format!("{HEX}FF8800orange{HEX}00ff00,000000green on black{HEX}reset")),
            ["\"orange\" fg=Rgb(16746496)", "\"green on black\" fg=Rgb(65280) bg=Rgb(0)", "\"reset\""]
        );
        // Not six hex digits: a plain reset, the rest is text.
        assert_eq!(seg(&format!("{HEX}12345x")), ["\"12345x\""]);
    }

    #[test]
    fn reverses_colours() {
        assert_eq!(seg(&format!("{R}rev")), ["\"rev\" fg=Background bg=Foreground"]);
        assert_eq!(seg(&format!("{C}4,2{R}rev")), ["\"rev\" fg=Palette(2) bg=Palette(4)"]);
    }

    #[test]
    fn drops_empty_segments() {
        assert!(seg(&format!("{B}{B}{C}4{C}")).is_empty());
    }

    #[test]
    fn strip_formatting_removes_every_control_code() {
        assert_eq!(strip_formatting(&format!("{B}bold{B} {C}04,12col{C} {HEX}ff0000,00ff00hex {I}{U}{S}{M}{R}x{O}")), "bold col hex x");
        assert_eq!(strip_formatting(&format!("{C}99,99plain")), "plain");
    }

    #[test]
    fn finds_http_www_and_mailto_links() {
        assert_eq!(links("see https://example.com/a?b=1&c=2#frag and http://x.org"), ["https://example.com/a?b=1&c=2#frag", "http://x.org"]);
        assert_eq!(links("www.example.com/path"), ["https://www.example.com/path"]);
        assert_eq!(links("mail mailto:ann@example.com"), ["mailto:ann@example.com"]);
    }

    #[test]
    fn trims_trailing_punctuation() {
        assert_eq!(links("go to https://example.com."), ["https://example.com"]);
        assert_eq!(links("https://example.com/a, then"), ["https://example.com/a"]);
        assert_eq!(links("really? https://example.com/q?!"), ["https://example.com/q"]);
        assert_eq!(links("\"https://example.com/quoted\""), ["https://example.com/quoted"]);
        assert_eq!(links("<https://example.com/angle>"), ["https://example.com/angle"]);
        assert_eq!(links("[https://example.com/sq]"), ["https://example.com/sq"]);
    }

    #[test]
    fn balances_parentheses() {
        assert_eq!(links("https://en.wikipedia.org/wiki/Foo_(bar)"), ["https://en.wikipedia.org/wiki/Foo_(bar)"]);
        assert_eq!(links("(see https://en.wikipedia.org/wiki/Foo_(bar))"), ["https://en.wikipedia.org/wiki/Foo_(bar)"]);
        assert_eq!(links("(https://example.com)"), ["https://example.com"]);
        assert_eq!(links("(https://example.com)."), ["https://example.com"]);
    }

    #[test]
    fn keeps_surrounding_text_and_formatting_around_a_link() {
        assert_eq!(seg(&format!("hi {B}https://a.io/x.{B} ok")), ["\"hi \"", "\"https://a.io/x\" bold href=https://a.io/x", "\".\" bold", "\" ok\""]);
    }

    #[test]
    fn needs_a_word_boundary_before_the_scheme() {
        assert!(links("xhttps://nope.example").is_empty());
        assert!(links("notwww.example.com").is_empty());
    }

    #[test]
    fn first_link_returns_the_first_previewable_link() {
        assert_eq!(first_link("mailto:a@b.c then (www.example.com).").as_deref(), Some("https://www.example.com"));
        assert_eq!(first_link(&format!("{C}4https://red.example{C}!")).as_deref(), Some("https://red.example"));
        assert_eq!(first_link("no links here"), None);
    }

    #[test]
    fn linkifies_channels_at_word_starts() {
        assert_eq!(chans("join #hearth, #rust-lang and (#svelte)"), ["#hearth", "#rust-lang", "#svelte"]);
        assert_eq!(chans("#start of line"), ["#start"]);
        assert_eq!(chans("see #hearth."), ["#hearth"]);
    }

    #[test]
    fn skips_mid_word_hashes_and_url_fragments() {
        assert!(chans("issue#12 c#").is_empty());
        assert_eq!(chans("https://example.com/ #real https://x.io/#frag"), ["#real"]);
        assert_eq!(links("https://example.com/ #real https://x.io/#frag"), ["https://example.com/", "https://x.io/#frag"]);
    }

    #[test]
    fn splits_the_text_around_the_channel() {
        assert_eq!(seg("go to #a now"), ["\"go to \"", "\"#a\" chan=#a", "\" now\""]);
    }

    #[test]
    fn mentions_match_whole_words_case_insensitively() {
        assert!(mentions("Ann: hi", &rules(&["ann"])));
        assert!(mentions("hi @ann!", &rules(&["ann"])));
        assert!(!mentions("annabelle", &rules(&["ann"])));
        assert!(!mentions("ann-marie", &rules(&["ann"])));
        assert!(!mentions("joann", &rules(&["ann"])));
    }

    #[test]
    fn mentions_handle_regex_metacharacters_and_nick_punctuation() {
        assert!(mentions("hey [ann]", &rules(&["[ann]"])));
        assert!(mentions("ping ann|away", &rules(&["ann|away"])));
        assert!(!mentions("ping ann", &rules(&["ann|away"])));
        assert!(mentions("I use c++", &rules(&["c++"])));
    }

    #[test]
    fn mentions_ignore_formatting_and_empty_words() {
        assert!(mentions(&format!("{B}a{B}nn"), &rules(&["ann"])));
        assert!(!mentions("anything", &rules(&[""])));
    }

    #[test]
    fn slash_rules_are_regexes_matched_as_written() {
        assert!(mentions("CI failed on main", &rules(&["/\\bci (failed|broke)\\b/i"])));
        assert!(!mentions("CI failed on main", &rules(&["/\\bci failed\\b/"])));
        assert!(mentions("deployed", &rules(&["/deploy(ed|ing)?/"])));
        let rule = rules(&["/x/g"]);
        assert_eq!([1, 2, 3].map(|_| mentions("x", &rule)), [true, true, true]);
    }

    #[test]
    fn reports_which_rule_matched_skipping_invalid_ones() {
        assert_eq!(matching_rule("the build is red", &rules(&["ann", "/(unclosed/", "/build/"])).map(String::as_str), Some("/build/"));
        assert_eq!(matching_rule(&format!("{B}ann{B}: hi"), &rules(&["bob", "ann"])).map(String::as_str), Some("ann"));
        assert_eq!(matching_rule("nothing here", &rules(&["ann"])), None);
        // Invalid flags stay invalid even though g is dropped when compiling.
        assert_eq!(matching_rule("foo", &rules(&["/foo/gg"])), None);
    }

    #[test]
    fn validates_regex_rules_and_accepts_plain_words() {
        assert_eq!(rule_error("deploy"), None);
        assert_eq!(rule_error("/a|b/i"), None);
        assert!(rule_error("/(unclosed/").is_some());
        assert!(rule_error("/a/q").unwrap().to_lowercase().contains("flag"));
    }
}
