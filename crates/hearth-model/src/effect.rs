//! What the model asks its host to do. The host drains these with
//! [`App::take_effects`](crate::App::take_effects), runs them against
//! `hearth_core::Core` (or the OS), and feeds the answers of requests back
//! with [`App::resolve`](crate::App::resolve).

use crate::types::{ServerConfig, Settings};
use hearth_core::certs::CertInfo;
use hearth_core::irc::message::Message as IrcMessage;
use hearth_core::preview::Preview;
use hearth_core::store::{SavedBuffer, SearchHit, SearchQuery};
use std::fmt;

/// Identifies a request, so its answer finds the code that was waiting for it.
pub type ReqId = u64;

/// A value that must never reach a log: `Debug` prints a placeholder.
#[derive(Clone, PartialEq)]
pub struct Secret<T>(pub T);

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Which kind of `Core::history` request this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Messages,
    Targets,
    Search,
}

impl HistoryKind {
    /// The `kind` argument `Core::history` takes.
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryKind::Messages => "messages",
            HistoryKind::Targets => "targets",
            HistoryKind::Search => "search",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Effect {
    // ── Commands: nothing to report back ──────────────────────
    /// `Core::send(conn, lines)`. If it fails, call `App::send_failed`.
    Send { conn: String, lines: Vec<String> },
    /// `Core::connect(server)`.
    Connect { server: String },
    /// `Core::disconnect(server, reason)`.
    Disconnect { server: String, reason: Option<String> },
    /// `Core::reconnect(server)`.
    Reconnect { server: String },
    /// `Core::reset()`: forget every connection (sent once at startup).
    Reset,
    /// `Core::delete_server(id)`.
    DeleteServer { id: String },
    /// `Core::save_settings(settings)`; already debounced.
    SaveSettings { settings: Settings },
    /// `Core::save_buffers(conn, buffers)`; already debounced.
    SaveBuffers { conn: String, buffers: Vec<SavedBuffer> },

    // ── Host (OS) integration ────────────────────────────────
    /// Show a system notification.
    Notify { title: String, body: String },
    /// Set the dock/taskbar badge to the number of mentions waiting.
    SetBadge { count: u32 },
    /// Open a link in the user's browser.
    OpenUrl { url: String },
    /// Put text on the clipboard.
    CopyText { text: String },
    /// Persist the open buffer's `App::active_key` to restore on next launch.
    RememberActive { key: String },

    // ── Requests: answer with `App::resolve(req, result)` ─────
    /// `Core::save_server`. Answer: `Reply::Server`.
    SaveServer { req: ReqId, server: Secret<ServerConfig> },
    /// `Core::load_buffers(conn)`. Answer: `Reply::Buffers`.
    LoadBuffers { req: ReqId, conn: String },
    /// `Core::history(conn, kind.as_str(), target, line)`. Answer: `Reply::Messages`.
    History { req: ReqId, conn: String, kind: HistoryKind, target: String, line: String },
    /// `Core::local_history(conn, target, before, after, limit)`; `target` is the folded buffer name. Answer: `Reply::Messages`.
    LocalHistory { req: ReqId, conn: String, target: String, before: Option<i64>, after: Option<i64>, limit: usize },
    /// `Core::search(query)`. Answer: `Reply::Hits`.
    Search { req: ReqId, query: SearchQuery },
    /// `Core::clear_logs()`. Answer: `Reply::Unit`.
    ClearLogs { req: ReqId },
    /// `Core::link_preview(url)`. Answer: `Reply::Preview`.
    LinkPreview { req: ReqId, url: String },
    /// `Core::avatar(url)`. Answer: `Reply::Text` (a data: URL).
    Avatar { req: ReqId, url: String },
    /// `Core::generate_cert(server)`. Answer: `Reply::Cert`.
    GenerateCert { req: ReqId, server: String },
    /// `Core::cert_info(path)`. Answer: `Reply::Cert`.
    CertInfo { req: ReqId, path: String },
    /// `Core::upload_bytes(server, url, name, content_type, data)`. Answer: `Reply::Text` (the uploaded file's URL).
    UploadBytes { req: ReqId, server: String, url: String, name: String, content_type: String, data: Secret<Vec<u8>> },
    /// `Core::upload_file(server, url, path)`. Answer: `Reply::Text`.
    UploadFile { req: ReqId, server: String, url: String, path: String },
}

impl Effect {
    /// The request id, for effects that expect an answer.
    pub fn req(&self) -> Option<ReqId> {
        match self {
            Effect::SaveServer { req, .. }
            | Effect::LoadBuffers { req, .. }
            | Effect::History { req, .. }
            | Effect::LocalHistory { req, .. }
            | Effect::Search { req, .. }
            | Effect::ClearLogs { req }
            | Effect::LinkPreview { req, .. }
            | Effect::Avatar { req, .. }
            | Effect::GenerateCert { req, .. }
            | Effect::CertInfo { req, .. }
            | Effect::UploadBytes { req, .. }
            | Effect::UploadFile { req, .. } => Some(*req),
            _ => None,
        }
    }
}

/// The successful answer to a request.
#[derive(Debug)]
pub enum Reply {
    Unit,
    Messages(Vec<IrcMessage>),
    Hits(Vec<SearchHit>),
    Buffers(Vec<SavedBuffer>),
    Preview(Preview),
    Text(String),
    Server(ServerConfig),
    Cert(CertInfo),
}
