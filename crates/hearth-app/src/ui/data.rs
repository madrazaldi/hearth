//! Plain view data and actions: the seam between the chat views and the state
//! (`hearth-model`, bound in `bind.rs`). The sidebar, header, member list and
//! message list read these structs and report user intent through `UiAction`;
//! they never mutate state. The overlays (search, switcher, settings, ...) are
//! bound to the host directly, since the model owns their state.

use super::prelude::*;
use hearth_model::timeline::Timeline;
use hearth_model::types::{ConnStatus, NotifyLevel};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

pub use hearth_core::certs::CertInfo as CertView;
pub use hearth_model::BufferKind;

/// The model's buffer identity.
pub type BufferId = hearth_model::BufId;

/// A person's picture, when they have one and it has loaded.
#[derive(Clone, Default)]
pub struct Pic(pub Option<ImageSource>);

impl fmt::Debug for Pic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() { "Pic(..)" } else { "Pic(none)" })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnState {
    Connected,
    /// Connecting or registering.
    Connecting,
    Disconnected,
}

impl From<ConnStatus> for ConnState {
    fn from(s: ConnStatus) -> Self {
        match s {
            ConnStatus::Connected => ConnState::Connected,
            ConnStatus::Disconnected => ConnState::Disconnected,
            _ => ConnState::Connecting,
        }
    }
}

// ── Sidebar ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct BufferRow {
    pub id: BufferId,
    pub kind: BufferKind,
    pub name: String,
    pub unread: usize,
    pub highlights: usize,
    pub level: Option<NotifyLevel>,
    pub joined: bool,
    /// Queries: `Some(true)` online, `Some(false)` offline, `None` unknown.
    pub online: Option<bool>,
    pub typing: bool,
    /// The person's picture (queries).
    pub picture: Pic,
}

impl BufferRow {
    pub fn muted(&self) -> bool {
        self.level == Some(NotifyLevel::None)
    }
}

#[derive(Clone, Debug)]
pub struct NetworkGroup {
    pub id: String,
    pub name: String,
    pub state: ConnState,
    /// "retrying in 13s", "connecting", or empty when connected.
    pub status_label: String,
    /// A bouncer's own entry: drawn as the small uppercase section label.
    pub is_control: bool,
    pub collapsed: bool,
    /// The server buffer, opened by clicking the row.
    pub server_buffer: BufferId,
    pub buffers: Vec<BufferRow>,
}

#[derive(Clone, Debug)]
pub struct FooterView {
    pub nick: String,
    /// "Online · Libera.Chat".
    pub status_text: String,
    pub state: ConnState,
    pub away: bool,
}

#[derive(Clone, Debug)]
pub struct SidebarView {
    pub networks: Vec<NetworkGroup>,
    pub active: Option<BufferId>,
    pub footer: Option<FooterView>,
    /// "⌘" on macOS, "Ctrl " elsewhere; used for the hint in "Jump to…".
    pub mod_label: String,
}

// ── Header ──────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct HeaderView {
    pub id: BufferId,
    pub kind: BufferKind,
    pub title: String,
    pub topic: Option<String>,
    /// "Set by nick · time", shown with the full topic.
    pub topic_note: Option<String>,
    pub subtitle: Option<String>,
    pub joined: bool,
    pub level: Option<NotifyLevel>,
    pub online: Option<bool>,
    pub member_count: usize,
    pub show_members: bool,
    pub picture: Pic,
}

// ── Messages ────────────────────────────────────────────────

/// A fetched or decoded picture with the size to reserve for it.
#[derive(Clone)]
pub struct ImageView {
    pub src: ImageSource,
    /// More than one frame: `still` is its first frame, shown while it is not playing.
    pub animated: bool,
    pub still: Option<ImageSource>,
    pub width: f32,
    pub height: f32,
}

impl fmt::Debug for ImageView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ImageView({}x{})", self.width, self.height)
    }
}

/// What the message list renders for one buffer. The lines themselves stay in
/// the model; rows read them from the host when they are painted.
#[derive(Clone)]
pub struct MessagesView {
    pub buffer: BufferId,
    pub timeline: Arc<Timeline>,
    /// Changes whenever any line does, so rows that grew can be measured again.
    pub lines_rev: u64,
    pub me: String,
    pub channel: bool,
    pub can_react: bool,
    /// The line to flash after a jump.
    pub flash: Option<u64>,
    /// The unread count shown on the "N new" pill.
    pub unread: usize,
    /// The text filter narrowing the list (trimmed, lower-cased). While it is set older history is
    /// not fetched and the buffer is not marked read.
    pub filter: String,
}

impl MessagesView {
    pub fn filtered(&self) -> bool {
        !self.filter.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct ReplyRef {
    pub nick: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ComposerView {
    pub buffer: BufferId,
    pub nick: String,
    pub placeholder: String,
    pub reply_to: Option<ReplyRef>,
    /// What the model has in the box: it changes it too (history recall, completion, emoji).
    pub draft: String,
    pub caret: usize,
    pub typing: Vec<String>,
    pub suggestions: hearth_model::composer::Suggestions,
    /// The server has a file host and this is a conversation: the paperclip shows.
    pub can_upload: bool,
    /// Uploads in flight for this buffer.
    pub uploading: u32,
}

// ── Actions ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum UiAction {
    OpenBuffer(BufferId),
    ToggleNetwork(String),
    CloseBuffer(BufferId),
    Rejoin(BufferId),
    MarkRead(BufferId),
    SetNotify(BufferId, Option<NotifyLevel>),
    OpenNick(String),
    OpenUrl(String),
    JoinChannel(String),
    Reply(u64),
    CancelReply,
    React(u64, String),
    CopyLine(u64),
    DeleteLine(u64),
    ToggleMembers,
    OpenSearch,
    /// Show the filter box in the header (Cmd/Ctrl+F does too).
    OpenFilter,
    CloseFilter,
    OpenSwitcher,
    AddNetwork,
    OpenSettings,
    SetSidebarWidth(f32),
    ShowTopic,
    OpenImage {
        url: String,
        image: ImageView,
    },

    // Nick menu
    OpenQuery(String),
    Mention(String),
    Whois(String),
    Watch(String, bool),
    Ignore(String, bool),
    ChannelMode {
        nick: String,
        mode: char,
        on: bool,
    },
    Kick(String),

    /// The dialog on screen was dismissed (Escape, the backdrop, its close button).
    DialogClosed(u64),
}

/// Receives user intent from any view.
pub type Dispatch = Rc<dyn Fn(UiAction, &mut Window, &mut App)>;

// ── Overlays ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    Servers,
    General,
    Appearance,
    Notifications,
    Aliases,
    Privacy,
    Shortcuts,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 7] = [
        SettingsTab::Servers,
        SettingsTab::General,
        SettingsTab::Appearance,
        SettingsTab::Notifications,
        SettingsTab::Aliases,
        SettingsTab::Privacy,
        SettingsTab::Shortcuts,
    ];

    /// The model's name for the tab.
    pub fn id(self) -> &'static str {
        match self {
            SettingsTab::Servers => "servers",
            SettingsTab::General => "general",
            SettingsTab::Appearance => "appearance",
            SettingsTab::Notifications => "notifications",
            SettingsTab::Aliases => "aliases",
            SettingsTab::Privacy => "privacy",
            SettingsTab::Shortcuts => "shortcuts",
        }
    }

    pub fn parse(s: &str) -> Option<SettingsTab> {
        SettingsTab::ALL.into_iter().find(|t| t.id() == s)
    }
}

// ── Settings ────────────────────────────────────────────────

/// One network of a soju bouncer.
#[derive(Clone, Debug)]
pub struct BouncerNetwork {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: String,
    pub tls: bool,
    pub nickname: String,
    pub username: String,
    pub state: ConnState,
    pub error: Option<String>,
}

/// A saved server with the state of its connection.
#[derive(Clone, Debug)]
pub struct ServerRow {
    pub config: hearth_model::types::ServerConfig,
    /// `None` when it has no connection.
    pub status: Option<ConnState>,
    pub is_bouncer: bool,
    pub networks: Vec<BouncerNetwork>,
}

/// What the settings pages show, read from the model.
#[derive(Clone, Debug)]
pub struct SettingsData {
    pub settings: hearth_model::types::Settings,
    pub servers: Vec<ServerRow>,
}

// Nick menu

#[derive(Clone, Debug)]
pub struct NickMenuView {
    pub nick: String,
    pub display_name: Option<String>,
    /// `user@host`.
    pub user_host: Option<String>,
    pub prefix: String,
    pub online: Option<bool>,
    pub away: bool,
    pub ignored: bool,
    pub account: Option<String>,
    pub realname: Option<String>,
    pub is_self: bool,
    pub can_monitor: bool,
    pub watched: bool,
    pub can_moderate: bool,
    pub is_op: bool,
    pub is_voice: bool,
    /// Show the avatar tile (profile pictures are on and the person has one).
    pub show_avatar: bool,
    pub picture: Pic,
}
