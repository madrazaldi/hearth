//! A scripted backend for developing and screenshotting the UI without a
//! network: the demo session (Libera.Chat, OFTC and Rizon through a soju
//! bouncer), a busy `#hearth`, link previews, and the first-run flow.
//!
//! ```ignore
//! let mut app = App::new();
//! let mut mock = hearth_model::mock::start(&mut app, MockOptions::default());
//! // every frame or so:
//! let host_effects = mock.drive(&mut app); // answers requests, delivers due events
//! ```
//!
//! `Stress::On` adds `#stress`, a channel with about 10k lines of scrollback
//! and 5k members; `Stress::Live` also posts a line every 3 seconds.

use crate::buffer::BufId;
use crate::conn::{ChannelInfo, ChannelList};
use crate::effect::{Effect, Reply, ReqId};
use crate::irc::iso_time;
use crate::state::App;
use crate::types::{IrcEvent, IrcMessage, Layout, SaslMechanism, SearchHit, ServerConfig, Settings, Source, Theme};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use hearth_core::config::Config;
use hearth_core::preview::Preview;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stress {
    #[default]
    Off,
    On,
    /// Also posts a line every 3 seconds.
    Live,
}

/// A view to open once the scene has loaded, for screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Switcher,
    Settings,
    Shortcuts,
    Search,
    Channels,
}

#[derive(Debug, Clone)]
pub struct MockOptions {
    /// Start with no servers: the first-run flow.
    pub empty: bool,
    pub stress: Stress,
    pub theme: Theme,
    pub layout: Layout,
    pub avatars: bool,
    pub view: Option<View>,
    /// The buffer to open (default `#hearth`, or `#stress`).
    pub buf: Option<String>,
    /// The search view's query.
    pub query: Option<String>,
    /// Open this settings tab (and server) once loaded.
    pub tab: Option<String>,
    pub server: Option<String>,
}

impl Default for MockOptions {
    fn default() -> Self {
        MockOptions {
            empty: false,
            stress: Stress::Off,
            theme: Theme::Dark,
            layout: Layout::Cozy,
            avatars: true,
            view: None,
            buf: None,
            query: None,
            tab: None,
            server: None,
        }
    }
}

// ── Messages ──────────────────────────────────────────────────

fn source(src: &str) -> Option<Source> {
    if src.is_empty() {
        return None;
    }
    let (nick, rest) = src.split_once('!').map_or((src, ""), |(n, r)| (n, r));
    let (user, host) = rest.split_once('@').map_or((rest, ""), |(u, h)| (u, h));
    let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
    Some(Source { nick: nick.to_string(), user: opt(user), host: opt(host) })
}

fn msg(src: &str, command: &str, params: &[&str], tags: &[(&str, &str)]) -> IrcMessage {
    IrcMessage {
        tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        source: source(src),
        command: command.into(),
        params: params.iter().map(|p| p.to_string()).collect(),
    }
}

fn with_tag(mut m: IrcMessage, k: &str, v: &str) -> IrcMessage {
    m.tags.insert(k.into(), v.into());
    m
}

fn user_src(nick: &str) -> String {
    format!("{nick}!{}@user/{}", nick.to_lowercase(), nick.to_lowercase())
}

// ── The demo scene ────────────────────────────────────────────

struct Scene {
    clock: i64,
    ev: Vec<IrcEvent>,
}

impl Scene {
    /// Advance the scripted clock by `mins` and give its time as a tag.
    fn at(&mut self, mins: i64) -> String {
        self.clock += mins * 60_000;
        iso_time(self.clock)
    }

    fn say(&mut self, nick: &str, target: &str, text: &str, mins: i64) -> IrcMessage {
        let t = self.at(mins);
        msg(&user_src(nick), "PRIVMSG", &[target, text], &[("time", &t)])
    }

    fn m(&mut self, conn: &str, x: IrcMessage) {
        self.ev.push(IrcEvent::Message { conn: conn.into(), msg: x });
    }

    /// A message with a `time` tag from the scripted clock.
    fn timed(&mut self, src: &str, command: &str, params: &[&str], mins: i64) -> IrcMessage {
        let t = self.at(mins);
        msg(src, command, params, &[("time", &t)])
    }
}

const NETS: [(&str, &str); 3] = [("1", "Libera.Chat"), ("2", "OFTC"), ("3", "Rizon")];

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn scene(t0: i64, stress: bool) -> Vec<IrcEvent> {
    let mut s = Scene { clock: t0, ev: Vec::new() };
    let soju = "soju".to_string();
    s.ev.push(IrcEvent::ConnAdded { conn: soju.clone(), server: soju.clone(), parent: None, netid: None });
    s.ev.push(IrcEvent::Caps { conn: soju.clone(), caps: strings(&["soju.im/bouncer-networks", "draft/chathistory"]), cap_values: BTreeMap::new() });
    s.ev.push(IrcEvent::Registered { conn: soju.clone(), nick: "ember".into(), caps: strings(&["soju.im/bouncer-networks"]), cap_values: BTreeMap::new() });
    s.ev.push(IrcEvent::Status { conn: soju.clone(), status: crate::types::ConnStatus::Connected, error: None, retry_in: None });
    for (id, name) in NETS {
        let attrs = format!("name={name};state=connected");
        s.m("soju", msg("bouncer.example", "BOUNCER", &["NETWORK", id, &attrs], &[]));
        let c = format!("soju/{id}");
        s.ev.push(IrcEvent::ConnAdded { conn: c.clone(), server: soju.clone(), parent: Some(soju.clone()), netid: Some(id.into()) });
        let caps = strings(&[
            "message-tags",
            "server-time",
            "echo-message",
            "away-notify",
            "multi-prefix",
            "draft/message-redaction",
            "labeled-response",
            "batch",
            "draft/multiline",
            "draft/metadata-2",
            "draft/channel-rename",
            "draft/account-registration",
            "standard-replies",
        ]);
        let cap_values: BTreeMap<String, String> = [("draft/multiline", "max-bytes=4096,max-lines=24"), ("draft/account-registration", "custom-account-name")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        s.ev.push(IrcEvent::Registered { conn: c.clone(), nick: "ember".into(), caps, cap_values });
        let isupport: BTreeMap<String, String> = [
            ("CHANTYPES", "#"),
            ("PREFIX", "(ov)@+"),
            ("CASEMAPPING", "rfc1459"),
            ("NETWORK", name),
            ("MONITOR", "100"),
            ("soju.im/FILEHOST", "https://files.hearth.example/upload"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        s.ev.push(IrcEvent::Isupport { conn: c.clone(), isupport });
        let down = id == "3";
        s.ev.push(IrcEvent::Status {
            conn: c,
            status: if down { crate::types::ConnStatus::Disconnected } else { crate::types::ConnStatus::Connected },
            error: down.then(|| "connection reset by peer".to_string()),
            retry_in: down.then_some(16),
        });
    }

    let (l, chan) = ("soju/1", "#hearth");
    let me = "ember!ember@user/ember";
    let lib = "irc.libera.chat";
    let j = s.timed(me, "JOIN", &[chan], 0);
    s.m(l, j);
    s.m(l, msg(lib, "332", &["ember", chan, "Hearth \u{2014} a quiet IRC client \u{b7} https://hearth.example \u{b7} Be kind, ship small"], &[]));
    let set = (t0 / 1000 - 86_400 * 3).to_string();
    s.m(l, msg(lib, "333", &["ember", chan, "mira", &set], &[]));
    let people = "@mira @tobias +wren +kestrel juniper oskar lin_ halvard sable quill rook ada pim felix noor ember";
    s.m(l, msg(lib, "353", &["ember", "=", chan, people], &[]));
    s.m(l, msg(lib, "366", &["ember", chan, "End of /NAMES list."], &[]));
    // draft/metadata-2: a few people have avatars and display names.
    for (nick, name) in [("mira", "Mira Okafor"), ("tobias", "Tobias"), ("wren", ""), ("juniper", "Juniper L."), ("sable", "")] {
        let url = format!("https://avatars.example/{nick}.png");
        s.m(l, msg(lib, "METADATA", &[nick, "avatar", "*", &url], &[]));
        if !name.is_empty() {
            s.m(l, msg(lib, "METADATA", &[nick, "display-name", "*", name], &[]));
        }
    }
    let x = s.say("mira", chan, "morning all \u{2014} the websocket transport landed on main last night", 2);
    s.m(l, with_tag(x, "msgid", "ws1"));
    let x = s.say("tobias", chan, "nice. does it negotiate text.ircv3.net or just assume it?", 1);
    s.m(l, x);
    let x = s.say("mira", chan, "negotiates, falls back to binary frames", 1);
    s.m(l, x);
    let x = s.say("mira", chan, "and lines over 4k get rejected before they hit the parser", 0);
    s.m(l, x);
    let x = s.timed("pim!pim@gateway/web", "JOIN", &[chan], 1);
    s.m(l, x);
    let x = s.timed("rook!rook@host", "QUIT", &["Ping timeout: 240 seconds"], 0);
    s.m(l, x);
    let x = s.timed("felix!felix@host", "PART", &[chan, "later"], 0);
    s.m(l, x);
    let x = s.say("wren", chan, "ember: did you get a chance to look at the scroll anchoring bug? WebKit still ignores overflow-anchor", 3);
    s.m(l, x);
    let x = s.say("ember", chan, "yeah \u{2014} ended up measuring scrollHeight before and after the prepend. no more jumping", 2);
    s.m(l, x);
    let x = s.timed(&user_src("kestrel"), "PRIVMSG", &[chan, "\x01ACTION puts the kettle on\x01"], 1);
    s.m(l, x);
    let x = s.say("juniper", chan, "bench numbers: \x02NAMES\x02 for a 2k-member channel now renders in \x0303~9ms\x03, down from 40", 4);
    s.m(l, x);
    let x = s.say("oskar", chan, "wait, are we coalescing the IPC events? that would explain it", 1);
    s.m(l, x);
    let x = s.say("juniper", chan, "yep, 8ms window, capped at 2000 events per payload", 1);
    s.m(l, x);
    // A draft/multiline message, as the backend delivers it: one message with line breaks.
    let x = s.say("oskar", chan, "repro steps for the reconnect bug:\n1. /raw QUIT\n2. watch it come back anyway\n\nfixed on the ircv3 branch", 1);
    s.m(l, with_tag(x, "msgid", "ml1"));
    let x = s.timed(lib, "FAIL", &["TOPIC", "NEED_MORE_PRIVS", chan, "You need channel operator status to change the topic"], 0);
    s.m(l, x);
    let x = s.timed("tobias!tobias@user/tobias", "MODE", &[chan, "+v", "juniper"], 1);
    s.m(l, x);
    let x = s.say("halvard", chan, "drafted the release notes: https://hearth.example/notes/0.1 \u{2014} shout if anything is wrong", 6);
    s.m(l, with_tag(x, "msgid", "rn1"));
    for (who, emoji) in [("tobias", "\u{1f389}"), ("wren", "\u{1f389}"), ("mira", "\u{1f440}")] {
        s.m(l, msg(&format!("{who}!{who}@user/{who}"), "TAGMSG", &[chan], &[("+draft/react", emoji), ("+draft/reply", "rn1")]));
    }
    let x = s.say("sable", chan, "the typing indicator is subtle in a good way", 1);
    s.m(l, with_tag(with_tag(x, "msgid", "ty1"), "+draft/reply", "ws1"));
    let x = s.say("oskar", chan, "new sidebar spacing, before/after: https://files.hearth.example/uploads/k3x9/sidebar.png", 2);
    s.m(l, x);
    let x = s.say("wren", chan, "kettle status https://media.example/kettle.gif", 1);
    s.m(l, x);
    let x = s.say("lin_", chan, "screen recording of the reconnect flow https://files.hearth.example/uploads/p2q1/reconnect-demo.mp4", 1);
    s.m(l, x);
    let x = s.say("quill", chan, "nsfw (not really, just a very tall screenshot) https://img.example/tall.jpg", 1);
    s.m(l, x);
    s.m(l, msg(lib, "730", &["ember", "mira!mira@user/mira"], &[]));

    let x = s.say("mira", "ember", "hey, can you review #42 when you have a minute?", 3);
    s.m(l, x);
    let x = s.say("ember", "mira", "on it after lunch", 1);
    s.m(l, x);
    let typing = |nick: &str, target: &str| msg(&format!("{nick}!{nick}@x"), "TAGMSG", &[target], &[("+typing", "active")]);
    s.m(l, typing("mira", "ember"));
    s.m(l, typing("oskar", chan));

    for (c, name) in [("#rust", "soju/1"), ("#svelte", "soju/1"), ("#bun", "soju/1"), ("#debian", "soju/2"), ("#oftc", "soju/2")] {
        let j = s.timed(me, "JOIN", &[c], 0);
        s.m(name, j);
        s.m(name, msg("srv", "353", &["ember", "=", c, "ember @op user1 user2"], &[]));
        s.m(name, msg("srv", "366", &["ember", c, "End"], &[]));
    }
    let x = s.say("ferris", "#rust", "anyone around who knows pin projection well?", 1);
    s.m("soju/1", x);
    let x = s.say("ferris", "#rust", "ember: you did the tokio actor thing, right?", 1);
    s.m("soju/1", x);
    let x = s.say("deb", "#debian", "trixie point release is out", 1);
    s.m("soju/2", x);
    // draft/channel-rename: #hearth-design becomes #hearth-ui, keeping its history.
    let j = s.timed(me, "JOIN", &["#hearth-design"], 0);
    s.m(l, j);
    s.m(l, msg("srv", "353", &["ember", "=", "#hearth-design", "ember @mira wren"], &[]));
    s.m(l, msg("srv", "366", &["ember", "#hearth-design", "End"], &[]));
    let x = s.say("wren", "#hearth-design", "mockups for the member list avatars are up", 1);
    s.m(l, x);
    let x = s.timed("mira!mira@user/mira", "RENAME", &["#hearth-design", "#hearth-ui", "design and UI talk"], 1);
    s.m(l, x);

    if stress {
        let j = s.timed(me, "JOIN", &["#stress"], 0);
        s.m(l, j);
        let names: Vec<String> = stress_nicks()
            .iter()
            .enumerate()
            .map(|(i, n)| {
                format!(
                    "{}{n}",
                    if i < 12 {
                        "@"
                    } else if i < 60 {
                        "+"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        for chunk in names.chunks(400) {
            let list = chunk.join(" ");
            s.m(l, msg("srv", "353", &["ember", "=", "#stress", &list], &[]));
        }
        s.m(l, msg("srv", "366", &["ember", "#stress", "End"], &[]));
    }
    s.ev
}

/// Older `#hearth` messages the bouncer would replay.
fn history(target: &str, t0: i64) -> Vec<IrcMessage> {
    if target != "#hearth" {
        return Vec::new();
    }
    let mut s = Scene { clock: t0 - 26 * 3_600_000, ev: Vec::new() };
    vec![
        s.say("mira", target, "proposal: we keep the palette warm but let the ember accent do all the work", 0),
        s.say("tobias", target, "agreed, no gradients on anything that is not a first-run screen", 2),
        s.say("lin_", target, "monospace only for timestamps and raw protocol stuff?", 3),
        s.say("mira", target, "exactly. Plex Sans for prose, Plex Mono for machine text", 1),
    ]
}

// ── ?stress ───────────────────────────────────────────────────
// Deterministic so a line's content doesn't change between runs.

const STRESS_TOTAL: i64 = 12_000;
const STRESS_FIRST: i64 = 10_000;
const NICK_STEMS: [&str; 16] =
    ["ash", "birch", "cedar", "dune", "elm", "fern", "gale", "heath", "iris", "juno", "kite", "lark", "moss", "nova", "onyx", "pike"];
const WORDS: &str = "the a scroll anchor websocket parser buffer render frame layout paint row height measure virtual list prepend history bouncer cap sasl batch label nick channel topic kettle tea patch review merge ship release build tokio actor webkit";

fn stress_nicks() -> Vec<String> {
    (0..5000usize)
        .map(|i| {
            if i == 0 {
                "ember".to_string()
            } else {
                format!("{}{}{}", NICK_STEMS[i % 16], if i % 7 == 0 { "_" } else { "" }, if i / 16 == 0 { String::new() } else { (i / 16).to_string() })
            }
        })
        .collect()
}

/// mulberry32, as the TypeScript mock had it.
fn rng(seed: u32) -> impl FnMut() -> f64 {
    let mut t = seed.wrapping_add(0x6d2b_79f5);
    move || {
        t = t.wrapping_add(0x6d2b_79f5);
        let mut r = (t ^ (t >> 15)).wrapping_mul(1 | t);
        r = r.wrapping_add((r ^ (r >> 7)).wrapping_mul(61 | r)) ^ r;
        f64::from(r ^ (r >> 14)) / 4_294_967_296.0
    }
}

/// Line `i` counts back from the newest (0).
fn stress_time(end: i64, i: i64) -> i64 {
    end - i * 31_000 - (i % 5) * 1000
}

fn stress_lines(end: i64, nicks: &[String], i: i64) -> Vec<IrcMessage> {
    let words: Vec<&str> = WORDS.split(' ').collect();
    let mut r = rng(i as u32);
    let pick = (r() * r() * 400.0) as usize;
    let nick = &nicks[1 + pick];
    let src = format!("{nick}!{nick}@user/{nick}");
    let time = iso_time(stress_time(end, i));
    let id = format!("s{i}");
    let mut tags: Vec<(String, String)> = vec![("time".into(), time), ("msgid".into(), id)];
    let k = r();
    let build = |command: &str, params: &[&str], tags: &[(String, String)]| IrcMessage {
        tags: tags.iter().cloned().collect(),
        source: source(&src),
        command: command.into(),
        params: params.iter().map(|p| p.to_string()).collect(),
    };
    if k < 0.05 {
        let command = ["JOIN", "PART", "QUIT"][(r() * 3.0) as usize];
        let params: &[&str] = if k < 0.017 { &["#stress"] } else { &["#stress", "bye"] };
        return vec![build(command, params, &tags)];
    }
    let span = if r() < 0.1 { 90.0 } else { 18.0 };
    let n = 3 + (r() * span) as usize;
    let mut text = (0..n).map(|_| words[(r() * words.len() as f64) as usize]).collect::<Vec<_>>().join(" ");
    if r() < 0.03 {
        text = format!("ember: {text}");
    }
    if r() < 0.02 {
        text += &format!(" https://hearth.example/notes/{i}");
    }
    // Not drawn from r(), so the rest of the line stays as it was.
    if i % 41 == 7 {
        text += &format!(" https://img.example/s/{i}.png");
    }
    if r() < 0.03 {
        tags.push(("+draft/reply".into(), format!("s{}", i + 1 + (r() * 4.0) as i64)));
    }
    let body = if (0.08..0.12).contains(&k) { format!("\x01ACTION {text}\x01") } else { format!("#{i} {text}") };
    let mut out = vec![build(if k < 0.08 { "NOTICE" } else { "PRIVMSG" }, &["#stress", &body], &tags)];
    if r() < 0.02 {
        let react = [("+draft/react".to_string(), "\u{1f44d}".to_string()), ("+draft/reply".to_string(), format!("s{i}"))];
        out.push(IrcMessage {
            tags: react.into_iter().collect(),
            source: source(&format!("{}!x@y", nicks[7])),
            command: "TAGMSG".into(),
            params: vec!["#stress".into()],
        });
    }
    out
}

fn stress_history(end: i64, before: Option<i64>, limit: usize) -> Vec<IrcMessage> {
    let nicks = stress_nicks();
    let (mut from, count) = match before {
        None => (0, STRESS_FIRST),
        Some(b) => {
            let mut f = (((end - b) as f64) / 31_000.0).ceil() as i64 - 1;
            f = f.max(0);
            while f < STRESS_TOTAL && stress_time(end, f) >= b {
                f += 1;
            }
            (f, limit as i64)
        }
    };
    from = from.max(0);
    let mut out = Vec::new();
    for i in (from..(from + count).min(STRESS_TOTAL)).rev() {
        out.extend(stress_lines(end, &nicks, i));
    }
    out
}

// ── Images ────────────────────────────────────────────────────
// What the backend would send as data URLs, drawn here as small PNGs.

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for b in data {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for d in data {
        a = (a + u32::from(*d)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// An uncompressed (stored) 8-bit RGB PNG.
fn png(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend(px(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend(b);
    }
    z.extend(adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn data_url(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(bytes))
}

fn hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let (h, s, l) = (h.rem_euclid(360.0) / 360.0, s / 100.0, l / 100.0);
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let ch = |mut t: f64| {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    [ch(h + 1.0 / 3.0), ch(h), ch(h - 1.0 / 3.0)]
}

/// A gradient with a few text-like bars. The picture is small whatever size
/// it claims, since the view scales it to the reported dimensions.
fn draw(w: u32, h: u32, hue: u32) -> String {
    let scale = 96.0 / f64::from(w.max(h));
    let (pw, ph) = (((f64::from(w) * scale) as u32).max(8), ((f64::from(h) * scale) as u32).max(8));
    let (a, b) = (hsl(f64::from(hue), 55.0, 38.0), hsl(f64::from(hue + 50), 60.0, 62.0));
    let bytes = png(pw, ph, |x, y| {
        let (fx, fy) = (f64::from(x) / f64::from(pw), f64::from(y) / f64::from(ph));
        let on_bar = (0..5).any(|i| {
            let top = 0.12 + f64::from(i) * 0.16;
            let len = 0.3 + f64::from((i * 37) % 50) / 100.0;
            fy >= top && fy < top + 0.07 && fx >= 0.08 && fx < 0.08 + len
        });
        let t = (fx + fy) / 2.0;
        let mix = |i: usize| (f64::from(a[i]) * (1.0 - t) + f64::from(b[i]) * t) as u8;
        let base = [mix(0), mix(1), mix(2)];
        if on_bar {
            base.map(|c| (f64::from(c) * 0.15 + 255.0 * 0.85) as u8)
        } else {
            base
        }
    });
    data_url(&bytes)
}

/// A stand-in for the backend's fetched avatar: a silhouette on the nick's hue.
fn mock_avatar(url: &str) -> String {
    let name = url.rsplit('/').next().unwrap_or(url).trim_end_matches(".png");
    let hue = name.chars().fold(7u32, |h, c| (h * 31 + c as u32) % 360);
    let (bg, fg) = (hsl(f64::from(hue), 55.0, 55.0), hsl(f64::from(hue), 60.0, 85.0));
    let bytes = png(40, 40, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let head = (fx - 20.0).powi(2) + (fy - 16.0).powi(2) <= 49.0;
        let body = ((fx - 20.0) / 13.0).powi(2) + ((fy - 36.0) / 10.0).powi(2) <= 1.0;
        if head || body {
            fg
        } else {
            bg
        }
    });
    data_url(&bytes)
}

fn preview(url: &str) -> Result<Preview, String> {
    let base = Preview {
        url: url.into(),
        kind: "image".into(),
        title: None,
        description: None,
        site: None,
        image: None,
        width: None,
        height: None,
        animated: false,
        mime: None,
        size: None,
    };
    let image = |w: u32, h: u32, hue: u32, mime: &str| Preview {
        image: Some(draw(w, h, hue)),
        width: Some(w),
        height: Some(h),
        mime: Some(mime.into()),
        size: Some(u64::from(w) * u64::from(h)),
        ..base.clone()
    };
    if url.contains("hearth.example/notes") {
        return Ok(Preview {
            kind: "page".into(),
            title: Some("Hearth 0.1 release notes".into()),
            description: Some("Local search, saved scrollback, replies and reactions, link previews, and a channel browser.".into()),
            site: Some("hearth.example".into()),
            image: Some(draw(240, 240, 20)),
            width: Some(240),
            height: Some(240),
            ..base
        });
    }
    if url.ends_with("sidebar.png") {
        return Ok(image(1600, 900, 24, "image/png"));
    }
    if url.ends_with("tall.jpg") {
        return Ok(image(700, 1400, 200, "image/jpeg"));
    }
    if url.ends_with("kettle.gif") {
        return Ok(Preview {
            image: Some(draw(200, 120, 30)),
            width: Some(200),
            height: Some(120),
            animated: true,
            mime: Some("image/gif".into()),
            size: Some(2809),
            ..base
        });
    }
    if url.ends_with(".mp4") {
        return Ok(Preview { kind: "media".into(), mime: Some("video/mp4".into()), size: Some(18_400_000), ..base });
    }
    if let Some(n) = url.split("img.example/s/").nth(1).and_then(|r| r.split(['.', '/']).next()).and_then(|n| n.parse::<u32>().ok()) {
        let k = (n % 4) as usize;
        return Ok(image([320, 800, 1200, 500][k], [480, 600, 300, 500][k], (n * 47) % 360, "image/png"));
    }
    Err("no preview".into())
}

// ── The backend ───────────────────────────────────────────────

#[allow(clippy::large_enum_variant)]
enum Due {
    Events(Vec<IrcEvent>),
    Answer(ReqId, Result<Reply, String>),
}

/// Answers the app's requests and plays the scripted session.
pub struct MockBackend {
    opts: MockOptions,
    /// The scene's time zero: 95 minutes ago.
    t0: i64,
    /// The newest `#stress` line.
    stress_end: i64,
    due: Vec<(i64, Due)>,
    last_saved: Option<ServerConfig>,
    script_at: Option<i64>,
    live_at: Option<i64>,
    live_n: u64,
}

/// The config the demo starts with.
pub fn config(opts: &MockOptions) -> Config {
    let mut s = Settings { theme: opts.theme, layout: opts.layout, link_previews: true, load_avatars: opts.avatars, ..Settings::default() };
    s.aliases = [("ns", "/msg NickServ $*"), ("j", "/join $1"), ("hi", "hello $1!; /me waves")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    s.network_highlights
        .insert("soju".into(), crate::types::NetworkHighlights { words: strings(&["release", "/\\bci (failed|broke)\\b/i"]), never: strings(&["*bot*"]) });
    let server = ServerConfig {
        id: "soju".into(),
        name: "soju".into(),
        host: "bouncer.example".into(),
        port: 443,
        websocket: true,
        nick: "ember".into(),
        ..ServerConfig::default()
    };
    Config { servers: if opts.empty { Vec::new() } else { vec![server] }, settings: s.to_value() }
}

/// Load the demo config into `app` and return the backend to drive it with.
pub fn start(app: &mut App, opts: MockOptions) -> MockBackend {
    let now = app.now();
    app.init(config(&opts), None);
    MockBackend {
        t0: now - 95 * 60_000,
        stress_end: now - 100 * 60_000,
        due: Vec::new(),
        last_saved: None,
        script_at: Some(now + 400),
        live_at: None,
        live_n: 0,
        opts,
    }
}

impl MockBackend {
    fn after(&mut self, now: i64, ms: i64, d: Due) {
        self.due.push((now + ms, d));
    }

    /// When the next scripted event is due, so the host can sleep until then.
    pub fn next_wakeup(&self) -> Option<i64> {
        self.due.iter().map(|(t, _)| *t).chain(self.script_at).chain(self.live_at).min()
    }

    /// Run the app's timers, answer its requests and deliver scripted events
    /// that are due. Returns the effects meant for the host (notifications,
    /// clipboard, badge, links, remembered buffer).
    pub fn drive(&mut self, app: &mut App) -> Vec<Effect> {
        let mut host = Vec::new();
        for _ in 0..16 {
            app.tick();
            let now = app.now();
            let mut busy = false;
            for e in app.take_effects() {
                busy = true;
                if let Some(e) = self.answer(app, now, e) {
                    host.push(e);
                }
            }
            if self.script_at.is_some_and(|t| t <= now) {
                self.script_at = None;
                self.run_script(app);
                busy = true;
            }
            if self.live_at.is_some_and(|t| t <= now) {
                self.live_n += 1;
                self.live_at = Some(now + 3000);
                let nick = &stress_nicks()[1 + (self.live_n as usize % 50)];
                let words: Vec<&str> = WORDS.split(' ').collect();
                let text = format!("live {}: {}", self.live_n, words[..(3 + (self.live_n as usize % 20)).min(words.len())].join(" "));
                let t = iso_time(now);
                let live = msg(&format!("{nick}!{nick}@user/{nick}"), "PRIVMSG", &["#stress", &text], &[("time", &t)]);
                app.handle_events(&[IrcEvent::Message { conn: "soju/1".into(), msg: live }]);
                busy = true;
            }
            let (ready, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.due).into_iter().partition(|(t, _)| *t <= now);
            self.due = later;
            for (_, d) in ready {
                busy = true;
                match d {
                    Due::Events(ev) => app.handle_events(&ev),
                    Due::Answer(req, r) => app.resolve(req, r),
                }
            }
            if !busy {
                break;
            }
        }
        host
    }

    /// Answer one effect; hands back the ones for the host.
    fn answer(&mut self, app: &mut App, now: i64, e: Effect) -> Option<Effect> {
        let ok = |app: &mut App, req, r| app.resolve(req, Ok(r));
        match e {
            Effect::Connect { server } => match self.last_saved.clone().filter(|s| s.id == server) {
                Some(s) => self.first_run(now, &s),
                None => {
                    let ev = scene(self.t0, self.opts.stress != Stress::Off);
                    self.after(now, 30, Due::Events(ev));
                    if self.opts.stress == Stress::Live {
                        self.live_at = Some(now + 3000);
                    }
                }
            },
            Effect::SaveServer { req, server } => {
                let saved = ServerConfig { id: "new".into(), ..server.0 };
                self.last_saved = Some(saved.clone());
                ok(app, req, Reply::Server(saved));
            }
            Effect::History { req, target, kind, .. } => {
                let msgs = if kind == crate::effect::HistoryKind::Messages { history(&target, self.t0) } else { Vec::new() };
                ok(app, req, Reply::Messages(msgs));
            }
            Effect::LocalHistory { req, target, before, after, limit, .. } => {
                let msgs = if self.opts.stress != Stress::Off && target == "#stress" && after.is_none() {
                    stress_history(self.stress_end, before, limit)
                } else {
                    Vec::new()
                };
                ok(app, req, Reply::Messages(msgs));
            }
            Effect::LoadBuffers { req, .. } => ok(app, req, Reply::Buffers(Vec::new())),
            Effect::Search { req, query } => {
                let hits = self.search(&query);
                ok(app, req, Reply::Hits(hits));
            }
            Effect::ClearLogs { req } => ok(app, req, Reply::Unit),
            Effect::LinkPreview { req, url } => self.after(now, 120, Due::Answer(req, preview(&url).map(Reply::Preview))),
            Effect::Avatar { req, url } => ok(app, req, Reply::Text(mock_avatar(&url))),
            Effect::CertInfo { req, .. } => app.resolve(req, Err("file not found".into())),
            Effect::GenerateCert { req, .. } => app.resolve(req, Err("certificates are not generated in the mock".into())),
            Effect::UploadBytes { req, name, .. } => ok(app, req, Reply::Text(format!("https://files.hearth.example/uploads/mock/{name}"))),
            Effect::UploadFile { req, path, .. } => {
                let name = path.rsplit(['/', '\\']).next().unwrap_or("file").to_string();
                ok(app, req, Reply::Text(format!("https://files.hearth.example/uploads/mock/{name}")));
            }
            Effect::Send { conn, lines } => self.echo(now, &conn, &lines),
            Effect::Notify { .. } | Effect::SetBadge { .. } | Effect::OpenUrl { .. } | Effect::CopyText { .. } | Effect::RememberActive { .. } => {
                return Some(e)
            }
            Effect::Disconnect { .. }
            | Effect::Reconnect { .. }
            | Effect::Reset
            | Effect::DeleteServer { .. }
            | Effect::SaveSettings { .. }
            | Effect::SaveBuffers { .. } => {}
        }
        None
    }

    /// The bouncer echoes what you say (the demo networks have echo-message).
    fn echo(&mut self, now: i64, conn: &str, lines: &[String]) {
        if !conn.starts_with("soju/") {
            return;
        }
        let events: Vec<IrcEvent> = lines
            .iter()
            .filter_map(|l| IrcMessage::parse(l))
            .filter(|m| matches!(m.command.as_str(), "PRIVMSG" | "NOTICE" | "TAGMSG"))
            .map(|mut m| {
                m.source = source("ember!ember@user/ember");
                m.tags.insert("time".into(), iso_time(now));
                m.tags.insert("msgid".into(), format!("echo{now}"));
                IrcEvent::Message { conn: conn.into(), msg: m }
            })
            .collect();
        if !events.is_empty() {
            self.after(now, 40, Due::Events(events));
        }
    }

    fn search(&self, q: &crate::types::SearchQuery) -> Vec<SearchHit> {
        let needle = q.text.to_lowercase();
        let mut all = history("#hearth", self.t0);
        let mut s = Scene { clock: self.t0 + 30 * 60_000, ev: Vec::new() };
        all.push(s.say("mira", "#hearth", "morning all \u{2014} the websocket transport landed on main last night", 0));
        if self.opts.stress != Stress::Off {
            all.extend(stress_history(self.stress_end, None, 0).into_iter().filter(|m| m.command == "PRIVMSG"));
        }
        let time = |m: &IrcMessage| m.tags.get("time").and_then(|t| crate::irc::parse_time(t)).unwrap_or(0);
        let hits: Vec<SearchHit> = all
            .into_iter()
            .filter(|m| m.param(1).to_lowercase().contains(&needle))
            .filter(|m| q.from.as_ref().is_none_or(|f| m.source.as_ref().is_some_and(|s| s.nick.eq_ignore_ascii_case(f))))
            .filter(|m| q.target.as_ref().is_none_or(|t| m.param(0).eq_ignore_ascii_case(t)))
            .filter(|m| q.before.is_none_or(|b| time(m) < b) && q.after.is_none_or(|a| time(m) > a))
            .map(|m| SearchHit { network: "soju/1".into(), target: m.param(0).to_string(), msg: m })
            .collect();
        let skip = hits.len().saturating_sub(200);
        hits.into_iter().skip(skip).collect()
    }

    /// The first-run flow: a scripted connection for the server just saved.
    /// A host containing "nowhere" fails DNS, SASL password "wrong" is
    /// rejected, and the nick "taken" collides.
    fn first_run(&mut self, now: i64, s: &ServerConfig) {
        let id = s.id.clone();
        self.after(
            now,
            20,
            Due::Events(vec![
                IrcEvent::ConnAdded { conn: id.clone(), server: id.clone(), parent: None, netid: None },
                IrcEvent::Status { conn: id.clone(), status: crate::types::ConnStatus::Connecting, error: None, retry_in: None },
            ]),
        );
        let status = |status, error: Option<String>, retry_in| IrcEvent::Status { conn: id.clone(), status, error, retry_in };
        if s.host.contains("nowhere") {
            let error = format!("could not reach {}:{}: failed to lookup address information: nodename nor servname provided", s.host, s.port);
            self.after(now, 900, Due::Events(vec![status(crate::types::ConnStatus::Disconnected, Some(error), Some(2))]));
            return;
        }
        self.after(now, 700, Due::Events(vec![status(crate::types::ConnStatus::Registering, None, None)]));
        let t = iso_time(now);
        if s.sasl == SaslMechanism::Plain && s.sasl_password == "wrong" {
            let m = msg(&s.host, "904", &[&s.nick, "SASL authentication failed"], &[("time", &t)]);
            self.after(now, 1300, Due::Events(vec![IrcEvent::Message { conn: id.clone(), msg: m }]));
            self.after(
                now,
                1500,
                Due::Events(vec![status(crate::types::ConnStatus::Disconnected, Some("Closing Link: (SASL authentication failed)".into()), Some(2))]),
            );
            return;
        }
        let nick = if s.nick == "taken" { "taken_".to_string() } else { s.nick.clone() };
        let welcome = format!("Welcome to {}, {nick}", if s.name.is_empty() { &s.host } else { &s.name });
        self.after(
            now,
            1800,
            Due::Events(vec![
                IrcEvent::Registered { conn: id.clone(), nick: nick.clone(), caps: strings(&["server-time", "message-tags"]), cap_values: BTreeMap::new() },
                IrcEvent::Message { conn: id.clone(), msg: msg(&s.host, "001", &[&nick, &welcome], &[]) },
                status(crate::types::ConnStatus::Connected, None, None),
            ]),
        );
    }

    /// The scripted interactions for screenshots.
    fn run_script(&mut self, app: &mut App) {
        let target = self.opts.buf.clone().unwrap_or_else(|| if self.opts.stress != Stress::Off { "#stress" } else { "#hearth" }.into());
        let id = app.conn("soju/1").and_then(|c| c.key_of(&target)).map(|k| BufId::new("soju/1", k));
        if let Some(id) = &id {
            app.open(id);
        }
        match self.opts.view {
            Some(View::Switcher) => app.toggle_switcher(),
            // Like Cmd+, in the app: the Appearance tab.
            Some(View::Settings) => app.open_settings("appearance", None),
            Some(View::Shortcuts) => app.toggle_shortcuts(),
            Some(View::Search) => app.open_search(self.opts.query.as_deref().unwrap_or("transport")),
            Some(View::Channels) => {
                if let Some(c) = app.conn_mut("soju/1") {
                    let item = |name: &str, users, topic: &str| ChannelInfo { name: name.into(), users, topic: topic.into() };
                    c.channel_list = Some(ChannelList {
                        loading: false,
                        at: self.t0,
                        items: vec![
                            item("#libera", 2210, "Welcome to Libera.Chat | https://libera.chat"),
                            item("#linux", 1874, "Linux help and discussion"),
                            item("#rust", 1203, "The Rust programming language | https://rust-lang.org"),
                            item("#hearth", 16, "Hearth \u{2014} a quiet IRC client"),
                        ],
                    });
                    app.browser_conn = Some("soju/1".into());
                }
            }
            None => {}
        }
        if let Some(tab) = &self.opts.tab {
            app.open_settings(tab, self.opts.server.as_deref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BufferKind;

    fn demo(opts: MockOptions) -> (App, MockBackend) {
        let mut app = App::new();
        app.set_now(1_790_000_000_000);
        let mock = start(&mut app, opts);
        (app, mock)
    }

    fn run(app: &mut App, mock: &mut MockBackend, ms: i64) {
        for _ in 0..(ms / 50) {
            app.set_now(app.now() + 50);
            mock.drive(app);
        }
    }

    #[test]
    fn plays_the_demo_session() {
        let (mut app, mut mock) = demo(MockOptions::default());
        run(&mut app, &mut mock, 1000);
        assert_eq!(app.ordered_conns().len(), 4);
        let id = app.active().cloned().expect("opens #hearth");
        assert_eq!(app.buffer(&id).unwrap().name, "#hearth");
        let b = app.buffer(&id).unwrap();
        assert_eq!(b.kind, BufferKind::Channel);
        assert_eq!(b.member_count(), 14, "16 names, minus rook (quit) and felix (part)");
        assert!(b.lines().iter().any(|l| l.msgid.as_deref() == Some("rn1") && !l.reactions.is_empty()));
        assert!(app.conn("soju/1").unwrap().buffer("#hearth-ui").is_some(), "channel rename applied");
    }

    #[test]
    fn answers_previews_and_avatars() {
        let (mut app, mut mock) = demo(MockOptions::default());
        run(&mut app, &mut mock, 1000);
        app.request_preview("https://hearth.example/notes/0.1");
        run(&mut app, &mut mock, 300);
        assert!(matches!(app.preview("https://hearth.example/notes/0.1"), crate::state::PreviewState::Ready(_)));
        app.request_avatar("https://avatars.example/mira.png");
        run(&mut app, &mut mock, 100);
        assert!(matches!(app.avatar("https://avatars.example/mira.png"), crate::state::AvatarState::Ready(_)));
    }

    #[test]
    fn stress_channel_has_scrollback_and_members() {
        let (mut app, mut mock) = demo(MockOptions { stress: Stress::On, ..MockOptions::default() });
        run(&mut app, &mut mock, 1000);
        let id = app.active().cloned().unwrap();
        let b = app.buffer(&id).unwrap();
        assert_eq!(b.name, "#stress");
        assert_eq!(b.member_count(), 5000);
        assert!(b.lines().len() > 5000, "{} lines", b.lines().len());
    }

    #[test]
    fn png_is_decodable_in_shape() {
        let url = draw(1600, 900, 24);
        let bytes = STANDARD.decode(url.trim_start_matches("data:image/png;base64,")).unwrap();
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 96);
    }

    #[test]
    fn first_run_fails_politely_for_unreachable_hosts() {
        let (mut app, mut mock) = demo(MockOptions { empty: true, ..MockOptions::default() });
        run(&mut app, &mut mock, 100);
        assert!(app.welcoming());
    }
}
