//! Hearth's engine: IRC connections, the message log, the config and the
//! secrets. The desktop app drives it through [`app::Core`], whose methods are
//! the command API, and receives [`irc::events::Event`] batches in return.
//!
//! The engine runs on its own tokio runtime ([`runtime`]), so the app's
//! executor can be anything.

pub mod app;
pub mod certs;
pub mod config;
pub mod irc;
pub mod paths;
pub mod preview;
pub mod runtime;
pub mod secrets;
pub mod store;
mod upload;

pub use app::{CmdResult, Core, Pending};

#[cfg(test)]
mod test_util {
    use std::path::{Path, PathBuf};

    /// A scratch directory removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("hearth-core-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
