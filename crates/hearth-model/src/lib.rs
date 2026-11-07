//! Hearth's client state, with no UI toolkit and no async runtime.
//!
//! [`App`] is a synchronous state machine. The host:
//!
//! 1. feeds it what the core reports with [`App::handle_events`], and the
//!    answers to requests with [`App::resolve`];
//! 2. calls [`App::tick`] when [`App::next_wakeup`] comes due (debounces,
//!    typing indicators, auto-away);
//! 3. drains [`App::take_effects`] and performs them against `hearth_core`
//!    or the OS;
//! 4. redraws when [`App::revision`] changed. A buffer's own revision is
//!    [`Buffer::rev`].
//!
//! Time is injected through [`App::set_now`]; until it is called the system
//! clock is used.

pub mod a11y;
pub mod aliases;
pub mod buffer;
pub mod commands;
pub mod composer;
pub mod conn;
pub mod effect;
pub mod emoji;
pub mod env;
pub mod format;
pub mod history;
pub mod ignore;
pub mod incoming;
pub mod irc;
pub mod media;
pub mod mock;
pub mod monitor;
pub mod multiline;
pub mod nickcolor;
pub mod notify;
pub mod onboarding;
pub mod panels;
pub mod rows;
pub mod saved;
pub mod search;
pub mod search_panel;
pub mod shortcuts;
pub mod state;
pub mod testing;
#[cfg(test)]
mod tests;
pub mod time;
pub mod timeline;
pub mod types;
pub mod unread;
pub mod welcome;

pub use buffer::{BufId, Buffer, BufferKind, Member};
pub use conn::Conn;
pub use effect::{Effect, HistoryKind, Reply, ReqId, Secret};
pub use state::App;
pub use types::{Line, LineType, Settings};
