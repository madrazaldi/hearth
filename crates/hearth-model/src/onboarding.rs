//! First-run helpers: network presets, input validation, and plain-language
//! explanations for the ways a first connection can fail.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetKind {
    Network,
    Bouncer,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub blurb: &'static str,
    pub kind: PresetKind,
    pub host: &'static str,
    pub port: u16,
    pub tls: bool,
    /// NICKLEN, when the network is known to be stricter than the usual 30.
    pub nick_len: Option<usize>,
    /// Whether services accept SASL; `Some(false)` skips the account step.
    pub sasl: Option<bool>,
    /// Shown on the account step.
    pub account_note: Option<&'static str>,
    pub channels: &'static [&'static str],
}

const fn preset(id: &'static str, label: &'static str, blurb: &'static str, host: &'static str) -> NetworkPreset {
    NetworkPreset { id, label, blurb, kind: PresetKind::Network, host, port: 6697, tls: true, nick_len: None, sasl: None, account_note: None, channels: &[] }
}

pub fn presets() -> Vec<NetworkPreset> {
    vec![
        NetworkPreset {
            nick_len: Some(16),
            sasl: Some(true),
            account_note: Some("Libera.Chat requires SASL when you connect from some address ranges, such as many VPNs, cloud hosts and Tor. It also protects a registered nickname and lets you into channels that only allow registered users."),
            channels: &["#libera"],
            ..preset("libera", "Libera.Chat", "Free and open-source communities", "irc.libera.chat")
        },
        NetworkPreset {
            sasl: Some(false),
            account_note: Some("OFTC doesn't use SASL. To use a registered nickname, identify with NickServ after connecting, or add a client certificate under Custom server."),
            channels: &["#oftc"],
            ..preset("oftc", "OFTC", "Debian, Tor and other free software projects", "irc.oftc.net")
        },
        NetworkPreset {
            nick_len: Some(9),
            sasl: Some(false),
            account_note: Some("EFnet has no nickname services, so there's no account to log in to."),
            channels: &["#efnet"],
            ..preset("efnet", "EFnet", "One of the oldest networks, no services", "irc.efnet.org")
        },
        NetworkPreset {
            sasl: Some(true),
            account_note: Some("Log in if you have registered your nickname with NickServ."),
            channels: &["#rizon"],
            ..preset("rizon", "Rizon", "Large general-purpose network", "irc.rizon.net")
        },
        NetworkPreset {
            sasl: Some(true),
            account_note: Some("Log in if you have registered your nickname with NickServ."),
            channels: &["#hackint"],
            ..preset("hackint", "hackint", "Hacker and maker communities", "irc.hackint.org")
        },
        NetworkPreset {
            kind: PresetKind::Bouncer,
            sasl: Some(true),
            account_note: Some("soju needs the username and password of your bouncer account."),
            ..preset("soju", "soju bouncer", "Stays connected for you and keeps history", "")
        },
        NetworkPreset { kind: PresetKind::Custom, ..preset("custom", "Custom server", "Any other server, with every option", "") },
    ]
}

const SPECIAL: &str = "[]\\`_^{|}";

/// Checks a nickname against the RFC 2812 grammar most servers still enforce.
pub fn nick_error(nick: &str, max_len: usize, network: &str) -> Option<String> {
    if nick.is_empty() {
        return Some("Choose a nickname.".into());
    }
    if nick.chars().any(char::is_whitespace) {
        return Some("Nicknames can't contain spaces.".into());
    }
    if nick.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return Some("Nicknames can't start with a digit or a dash.".into());
    }
    if let Some(bad) = nick.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || SPECIAL.contains(*c))) {
        return Some(format!("“{bad}” isn't allowed. Use letters, digits and - _ [ ] {{ }} \\ | ^ `"));
    }
    if nick.chars().count() > max_len {
        return Some(format!("Keep it to {max_len} characters or fewer on {network}."));
    }
    None
}

/// Splits a channel list typed by hand; bare names get a `#`.
pub fn parse_channels(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || c == ',') {
        let c = raw.trim();
        if c.is_empty() {
            continue;
        }
        let name = if c.starts_with(['#', '&', '+', '!']) { c.to_string() } else { format!("#{c}") };
        if !out.iter().any(|x| x.to_lowercase() == name.to_lowercase()) {
            out.push(name);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friendly {
    pub title: String,
    pub hint: String,
}

pub struct ErrorContext<'a> {
    pub host: &'a str,
    pub port: u16,
    pub tls: bool,
    pub sasl: bool,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

/// Turns a backend connection error (an anyhow chain, or the server's ERROR
/// reason) into something a newcomer can act on. The raw text is still shown
/// alongside for anyone who wants it.
pub fn explain_error(raw: &str, ctx: &ErrorContext) -> Friendly {
    static SASL: OnceLock<Regex> = OnceLock::new();
    static DNS: OnceLock<Regex> = OnceLock::new();
    static TLS: OnceLock<Regex> = OnceLock::new();
    static UNREACH: OnceLock<Regex> = OnceLock::new();
    static LINE: OnceLock<Regex> = OnceLock::new();
    static THROTTLE: OnceLock<Regex> = OnceLock::new();
    let e = raw.to_lowercase();
    let f = |title: &str, hint: &str| Friendly { title: title.into(), hint: hint.into() };
    if re(&SASL, r"sasl|authenticat").is_match(&e) {
        return if ctx.sasl {
            f("Your account name or password was not accepted", "Check them on the account step and try again.")
        } else {
            f(
                "This network wants you to log in first",
                "Connections from your address need an account (SASL). Add your account on the account step, or connect from another network.",
            )
        };
    }
    if re(&DNS, r"lookup|resolve|nodename|name or service|no such host|dns").is_match(&e) {
        return Friendly { title: format!("Couldn't find {}", ctx.host), hint: "Check the server address for typos.".into() };
    }
    if e.contains("refused") {
        return Friendly {
            title: "The server refused the connection".into(),
            hint: format!("Nothing is accepting connections on port {}. Check the port, and whether the server expects TLS.", ctx.port),
        };
    }
    if re(&TLS, r"certificate|tls|handshake|unknownissuer|notvalidforname|expired").is_match(&e) {
        return Friendly {
            title: "Couldn't set up a secure connection".into(),
            hint: if ctx.tls {
                format!("The server's certificate wasn't trusted, or port {} doesn't speak TLS. TLS ports are usually 6697.", ctx.port)
            } else {
                "The server may require TLS. Turn TLS on and use port 6697.".into()
            },
        };
    }
    if re(&UNREACH, r"timed out connecting|unreachable|network is down|no route").is_match(&e) {
        return f("The server didn't answer", "Check your internet connection and the server address.");
    }
    if e.contains("registration timed out") {
        return f("The server never finished logging you in", "It may be busy. Try again in a moment.");
    }
    if re(&LINE, r"\b[kgzd]-?line|\bbanned\b|\bban\b").is_match(&e) {
        return f("This network is refusing connections from your address", "This is set by the network, not Hearth.");
    }
    if re(&THROTTLE, r"too many|throttl|reconnecting too fast").is_match(&e) {
        return f("Too many connections from your address", "Wait a minute, then try again.");
    }
    f("Couldn't connect", "The details below may help.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nick_error_accepts_ordinary_and_special_character_nicks() {
        for n in ["ember", "Ember_", "[away]", "a-b", "x^y", "`q", "{z}|w\\"] {
            assert_eq!(nick_error(n, 30, "this network"), None, "{n}");
        }
    }

    #[test]
    fn rejects_empty_spaced_and_badly_started_nicks() {
        let e = |n: &str| nick_error(n, 30, "this network").unwrap();
        assert!(e("").contains("Choose"));
        assert!(e("my nick").contains("spaces"));
        assert!(e("9lives").contains("digit"));
        assert!(e("-dash").contains("digit or a dash"));
    }

    #[test]
    fn names_the_offending_character() {
        assert!(nick_error("ember!", 30, "x").unwrap().contains("“!”"));
        assert!(nick_error("émile", 30, "x").unwrap().contains("“é”"));
    }

    #[test]
    fn enforces_the_network_nicklen() {
        assert!(nick_error("abcdefghij", 9, "EFnet").unwrap().contains("9 characters or fewer on EFnet"));
        assert_eq!(nick_error("abcdefghi", 9, "x"), None);
    }

    #[test]
    fn parse_channels_adds_hash_and_drops_duplicates() {
        assert_eq!(parse_channels("#libera, rust  #Rust\n&local"), ["#libera", "#rust", "&local"]);
        assert!(parse_channels("  ").is_empty());
    }

    fn ctx(sasl: bool) -> ErrorContext<'static> {
        ErrorContext { host: "irc.example", port: 6697, tls: true, sasl }
    }

    #[test]
    fn explains_common_transport_failures() {
        let t = |raw: &str| explain_error(raw, &ctx(false)).title;
        assert!(t("could not reach irc.example:6697: failed to lookup address information").contains("find irc.example"));
        assert!(t("could not reach irc.example:6697: Connection refused (os error 61)").contains("refused"));
        assert!(t("TLS handshake failed: invalid peer certificate: UnknownIssuer").contains("secure"));
        assert!(t("timed out connecting to irc.example:6697").contains("didn't answer"));
    }

    #[test]
    fn recognises_bans_without_mistaking_deadline_for_a_d_line() {
        assert!(explain_error("Closing Link: irc.example (K-Lined)", &ctx(false)).title.contains("refusing"));
        assert_eq!(explain_error("deadline has elapsed", &ctx(false)).title, "Couldn't connect");
    }

    #[test]
    fn tells_sasl_required_networks_apart_from_bad_credentials() {
        assert!(explain_error("Closing Link: (SASL access only)", &ctx(false)).title.contains("log in first"));
        assert!(explain_error("SASL authentication failed", &ctx(true)).title.contains("not accepted"));
        assert_eq!(explain_error("something odd", &ctx(false)).title, "Couldn't connect");
    }

    #[test]
    fn presets_have_tls_defaults_on_the_standard_port() {
        for p in presets().iter().filter(|p| p.kind == PresetKind::Network) {
            assert!(p.host.contains('.'));
            assert!(p.tls);
            assert_eq!(p.port, 6697);
        }
    }
}
