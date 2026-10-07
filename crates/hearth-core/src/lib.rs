//! Hearth's engine. The Tauri frontend drives it through [`app::Core`],
//! whose methods are the command API.

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
