//! Types the client state shares with the UI and, for settings, with the
//! backend (which reads a few keys out of the saved JSON blob).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use hearth_core::config::{Config, SaslMechanism, ServerConfig};
pub use hearth_core::irc::events::{Event as IrcEvent, Status as ConnStatus};
pub use hearth_core::irc::message::{Message as IrcMessage, Source};
pub use hearth_core::preview::Preview;
pub use hearth_core::store::{SavedBuffer, SearchHit, SearchQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    #[default]
    All,
    Mentions,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

/// The message layout. The UI calls `Cozy` "modern".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Cozy,
    Columns,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Clock {
    #[default]
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "12h")]
    H12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Membership {
    Show,
    #[default]
    Collapse,
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NetworkHighlights {
    /// Words, or `/regex/flags`.
    pub words: Vec<String>,
    /// Nicks or masks whose messages never highlight (bots, relays).
    pub never: Vec<String>,
}

/// The settings blob. Unknown keys are dropped and missing keys take their
/// defaults, so older and newer config files both load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    pub layout: Layout,
    pub font_size: u32,
    pub clock: Clock,
    pub show_seconds: bool,
    pub membership: Membership,
    pub nick_colors: bool,
    pub show_members: bool,
    pub notify_highlights: bool,
    pub notify_private: bool,
    pub highlight_words: Vec<String>,
    pub send_typing: bool,
    pub show_typing: bool,
    pub sidebar_width: u32,
    /// Keep a local copy of messages (offline scrollback and search).
    pub store_logs: bool,
    /// Fetch link and image previews (through the backend).
    pub link_previews: bool,
    /// Fetch avatars people set with draft/metadata-2.
    pub load_avatars: bool,
    /// With link previews on, show direct image links as images under the message.
    pub inline_images: bool,
    /// Play animated images without hovering.
    pub autoplay_animations: bool,
    pub close_to_tray: bool,
    /// Check the release feed on launch and every few hours.
    pub check_updates: bool,
    /// Mark away after this many minutes with the window in the background; 0 disables.
    pub auto_away_minutes: u32,
    /// nick or nick!user@host masks, `*` and `?` wildcards.
    pub ignores: Vec<String>,
    /// Per-buffer notification level, keyed by `Buffer::pref_key`.
    pub buffer_notify: BTreeMap<String, NotifyLevel>,
    /// Nicks to watch with MONITOR, keyed by `Conn::pref_key`.
    pub watch: BTreeMap<String, Vec<String>>,
    /// Extra highlight rules per server id; bouncer networks share their bouncer's.
    pub network_highlights: BTreeMap<String, NetworkHighlights>,
    /// User slash commands: name (no slash) → `;`-separated commands.
    pub aliases: BTreeMap<String, String>,
    pub spellcheck: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            layout: Layout::Cozy,
            font_size: 14,
            clock: Clock::H24,
            show_seconds: false,
            membership: Membership::Collapse,
            nick_colors: true,
            show_members: true,
            notify_highlights: true,
            notify_private: true,
            highlight_words: Vec::new(),
            send_typing: true,
            show_typing: true,
            sidebar_width: 248,
            store_logs: true,
            link_previews: false,
            load_avatars: false,
            inline_images: true,
            autoplay_animations: false,
            close_to_tray: false,
            check_updates: true,
            auto_away_minutes: 0,
            ignores: Vec::new(),
            buffer_notify: BTreeMap::new(),
            watch: BTreeMap::new(),
            network_highlights: BTreeMap::new(),
            aliases: BTreeMap::new(),
            spellcheck: true,
        }
    }
}

impl Settings {
    /// Settings from the saved blob; anything unreadable falls back to defaults.
    pub fn from_value(v: &serde_json::Value) -> Settings {
        if v.is_null() {
            return Settings::default();
        }
        serde_json::from_value(v.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("settings serialize")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineType {
    Privmsg,
    Action,
    Notice,
    Join,
    Part,
    Quit,
    Kick,
    Nick,
    Mode,
    Topic,
    Invite,
    Info,
    Error,
    Whois,
}

impl LineType {
    /// Join/part/quit/nick: the lines the `membership` setting collapses.
    pub fn is_membership(self) -> bool {
        matches!(self, LineType::Join | LineType::Part | LineType::Quit | LineType::Nick)
    }
}

/// One rendered line in a buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub id: u64,
    /// Milliseconds since the Unix epoch.
    pub time: i64,
    #[serde(rename = "type")]
    pub kind: LineType,
    pub nick: String,
    pub text: String,
    #[serde(rename = "self")]
    pub is_self: bool,
    pub highlight: bool,
    pub msgid: Option<String>,
    /// Extra subject: kicked nick, new nick, whois payload, …
    pub target: Option<String>,
    pub account: Option<String>,
    /// The sender's highest channel prefix (@, %, +, …) when the line arrived.
    pub prefix: Option<String>,
    pub redacted: bool,
    pub data: Option<BTreeMap<String, String>>,
    /// msgid of the message this one replies to (+draft/reply).
    pub reply_to: Option<String>,
    /// emoji → nicks who reacted (+draft/react), in first-reaction order.
    pub reactions: Vec<(String, Vec<String>)>,
}

impl Line {
    pub fn new(id: u64, time: i64, kind: LineType, nick: impl Into<String>, text: impl Into<String>) -> Line {
        Line {
            id,
            time,
            kind,
            nick: nick.into(),
            text: text.into(),
            is_self: false,
            highlight: false,
            msgid: None,
            target: None,
            account: None,
            prefix: None,
            redacted: false,
            data: None,
            reply_to: None,
            reactions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_tolerate_partial_blobs() {
        let s = Settings::from_value(&serde_json::json!({ "layout": "columns", "fontSize": 16, "bogus": 1 }));
        assert_eq!(s.layout, Layout::Columns);
        assert_eq!(s.font_size, 16);
        assert!(s.store_logs);
        assert_eq!(Settings::from_value(&s.to_value()), s);
        assert_eq!(Settings::from_value(&serde_json::json!({ "layout": 3 })), Settings::default());
    }
}
