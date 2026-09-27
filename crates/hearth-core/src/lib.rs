//! Hearth's engine. Frontends (Tauri + Svelte on Windows and Linux, the
//! SwiftUI app on macOS) drive it through [`app::Core`], whose methods are the
//! command API both of them share.

pub mod app;
pub mod certs;
pub mod config;
pub mod irc;
pub mod preview;
pub mod runtime;
pub mod store;
mod secrets;
mod upload;

pub use app::Core;
