//! Self-update from GitHub Releases.
//!
//! The release workflow publishes `latest.json` next to the installers, which
//! `releases/latest/download/` serves for the newest *published* release (a
//! draft is invisible, as with the old updater). The manifest lists, per
//! platform, an asset and its minisign signature. A download is installed only
//! if the signature verifies against the public key compiled into this build
//! (`HEARTH_UPDATER_PUBKEY`), so a compromised mirror or release page cannot
//! push code. The private key exists only in CI secrets.
//!
//! Installing is delegated to `cargo-packager-updater`: it swaps the `.app`
//! bundle (macOS), the AppImage (Linux), or runs the NSIS installer (Windows).
//!
//! Everything blocking runs on the engine's tokio runtime, never on GPUI's
//! main thread.

use anyhow::{Context as _, anyhow};
use cargo_packager_updater::semver::Version;
use cargo_packager_updater::url::Url;
use cargo_packager_updater::{Config, Update, UpdaterBuilder};
use gpui_kit::AsyncApp;
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_ENDPOINT: &str = "https://github.com/madrazaldi/hearth/releases/latest/download/latest.json";
/// The base64 minisign public key, set by the release build. Without one the
/// updater is off, so local and fork builds never trust an unknown signer.
const PUBKEY: Option<&str> = option_env!("HEARTH_UPDATER_PUBKEY");

/// The manifest is small; the installer is not.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// How often `download` reports progress, so a fast link does not flood the UI.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    pub version: String,
    /// Release notes (markdown), when the release has any.
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    /// `None` while the server has not said how big the file is.
    pub total: Option<u64>,
}

#[derive(Clone)]
struct Settings {
    endpoint: Url,
    pubkey: String,
    current: Version,
    /// The app to replace; the running executable's when `None`.
    executable: Option<PathBuf>,
}

#[derive(Default)]
struct Staged {
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
}

/// A cheap, cloneable handle; clones share the checked and downloaded update.
#[derive(Clone)]
pub struct Updater {
    settings: Result<Settings, &'static str>,
    staged: Arc<Mutex<Staged>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

impl Updater {
    /// The updater for this build of the app.
    pub fn new() -> Self {
        Self { settings: detect(), staged: Default::default() }
    }

    /// An updater for any feed and key; for tests and tools.
    pub fn with_feed(endpoint: Url, pubkey: &str, current: Version) -> Self {
        Self { settings: Ok(Settings { endpoint, pubkey: pubkey.into(), current, executable: None }), staged: Default::default() }
    }

    /// Replace the app at this path instead of the running executable.
    pub fn with_executable(mut self, path: PathBuf) -> Self {
        if let Ok(settings) = &mut self.settings {
            settings.executable = Some(path);
        }
        self
    }

    /// Why updates are off in this build or install, for the settings page to
    /// show instead of a button that cannot work.
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        self.settings.as_ref().err().copied()
    }

    /// Looks for a newer published release. Forgets any earlier download.
    pub async fn check(&self) -> anyhow::Result<Option<UpdateOffer>> {
        let settings = self.settings()?.clone();
        let found = hearth_core::runtime::blocking(move || -> anyhow::Result<Option<Update>> {
            let mut builder = UpdaterBuilder::new(settings.current, Config { endpoints: vec![settings.endpoint], pubkey: settings.pubkey, windows: None })
                .timeout(CHECK_TIMEOUT);
            if let Some(executable) = settings.executable {
                builder = builder.executable_path(executable);
            }
            let updater = builder.build().context("could not set up the update check")?;
            updater.check().context("could not check for updates")
        })
        .await
        .map_err(|e| anyhow!(e))??;

        let mut staged = self.staged.lock().unwrap();
        *staged = Staged::default();
        Ok(found.map(|update| {
            let offer = UpdateOffer { version: update.version.clone(), notes: update.body.clone() };
            staged.update = Some(update);
            offer
        }))
    }

    /// Downloads the offered update and checks its signature. `progress` is
    /// called from a worker thread.
    pub async fn download(&self, progress: impl Fn(Progress) + Send + Sync + 'static) -> anyhow::Result<()> {
        let mut update = self.staged.lock().unwrap().update.clone().ok_or_else(|| anyhow!("there is no update to download; check first"))?;
        update.timeout = Some(DOWNLOAD_TIMEOUT);
        let bytes = hearth_core::runtime::blocking(move || -> anyhow::Result<Vec<u8>> {
            let downloaded = Cell::new(0u64);
            let last = Cell::new(Instant::now() - PROGRESS_EVERY);
            let bytes = update.download_extended(
                |chunk, total| {
                    downloaded.set(downloaded.get() + chunk as u64);
                    if last.get().elapsed() >= PROGRESS_EVERY {
                        last.set(Instant::now());
                        progress(Progress { downloaded: downloaded.get(), total });
                    }
                },
                || {},
            )?;
            progress(Progress { downloaded: bytes.len() as u64, total: Some(bytes.len() as u64) });
            Ok(bytes)
        })
        .await
        .map_err(|e| anyhow!(e))?
        .context("could not download the update")?;
        self.staged.lock().unwrap().bytes = Some(bytes);
        Ok(())
    }

    /// Installs the downloaded update and starts the new version. The servers
    /// get their QUIT first; if installing fails afterwards, the connections
    /// are already closed and the caller should say so (a restart reconnects).
    pub async fn apply_and_relaunch(&self, cx: &mut AsyncApp) -> anyhow::Result<()> {
        let (update, bytes) = {
            let mut staged = self.staged.lock().unwrap();
            match (staged.update.clone(), staged.bytes.take()) {
                (Some(update), Some(bytes)) => (update, bytes),
                _ => return Err(anyhow!("the update has not been downloaded")),
            }
        };

        // The Windows installer ends this process (and closes any left open), so the
        // goodbye must come first there. Elsewhere installing can fail, and then the
        // app should still be connected.
        #[cfg(windows)]
        super::shutdown_connections(cx).await;
        hearth_core::runtime::blocking(move || update.install(bytes)).await.map_err(|e| anyhow!(e))?.context("could not install the update")?;
        #[cfg(not(windows))]
        {
            super::shutdown_connections(cx).await;
            cx.update(super::relaunch);
        }
        Ok(())
    }

    fn settings(&self) -> anyhow::Result<&Settings> {
        self.settings.as_ref().map_err(|reason| anyhow!("updates are unavailable: {reason}"))
    }
}

fn detect() -> Result<Settings, &'static str> {
    let pubkey = PUBKEY.map(str::trim).filter(|k| !k.is_empty()).ok_or("this build has no update signing key")?;
    if cfg!(debug_assertions) {
        return Err("updates are off in development builds");
    }
    updatable_install()?;
    // For testing a release candidate; the signature check still applies.
    let endpoint = std::env::var("HEARTH_UPDATE_URL").unwrap_or_else(|_| DEFAULT_ENDPOINT.into());
    Ok(Settings {
        endpoint: endpoint.parse().map_err(|_| "the update address is not a valid URL")?,
        pubkey: pubkey.into(),
        current: parse_version(super::version()).ok_or("the app version is not a semantic version")?,
        executable: None,
    })
}

/// Self-update replaces the app where it lives, which only makes sense for the
/// installs the release produces.
fn updatable_install() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        // Anywhere else the updater would treat the executable's folder as the app and replace it.
        let exe = std::env::current_exe().map_err(|_| "cannot find the running app")?;
        if !exe.to_string_lossy().contains(".app/Contents/MacOS/") {
            return Err("not running from an app bundle");
        }
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        return Err("not running from an AppImage; update through the package you installed");
    }
    Ok(())
}

pub(crate) fn parse_version(s: &str) -> Option<Version> {
    Version::parse(s.trim_start_matches('v')).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use std::io::{Cursor, Read, Write};
    use std::net::TcpListener;

    const PAYLOAD: &[u8] = b"pretend this is an installer";

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    /// Serves `/latest.json` and `/pkg` on localhost; returns the address.
    fn serve(manifest: impl Fn(&str) -> String + Send + 'static, payload: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = base.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = [0u8; 2048];
                let n = stream.read(&mut request).unwrap_or(0);
                let first_line = String::from_utf8_lossy(&request[..n]).lines().next().unwrap_or("").to_string();
                let body = if first_line.contains("/pkg") { payload.clone() } else { manifest(&served).into_bytes() };
                let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&body));
            }
        });
        base
    }

    struct Keys {
        public: String,
        signature: String,
    }

    fn sign(payload: &[u8]) -> Keys {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(Some(&pair.pk), &pair.sk, Cursor::new(payload), Some("test"), None).unwrap().to_string();
        Keys { public: b64(&pair.pk.to_box().unwrap().to_string()), signature: b64(&signature) }
    }

    fn manifest(version: &str, base: &str, signature: &str) -> String {
        let target = cargo_packager_updater::target().expect("a supported platform");
        let format = if cfg!(target_os = "macos") {
            "app"
        } else if cfg!(windows) {
            "nsis"
        } else {
            "appimage"
        };
        serde_json::json!({
            "version": version,
            "notes": "- something new",
            "platforms": { target: { "url": format!("{base}/pkg"), "signature": signature, "format": format } },
        })
        .to_string()
    }

    /// An updater on a feed, pretending to be installed somewhere without symlinks in its path.
    fn updater_for(base: &str, public: &str) -> Updater {
        Updater::with_feed(format!("{base}/latest.json").parse().unwrap(), public, Version::new(0, 1, 0))
            .with_executable(PathBuf::from("/Applications/Hearth.app/Contents/MacOS/hearth"))
    }

    fn run<T>(f: impl Future<Output = T>) -> T {
        hearth_core::runtime::handle().block_on(f)
    }

    #[test]
    fn offers_a_newer_release_and_downloads_it_verified() {
        let keys = sign(PAYLOAD);
        let signature = keys.signature.clone();
        let base = serve(move |base| manifest("9.9.9", base, &signature), PAYLOAD.to_vec());
        let updater = updater_for(&base, &keys.public);

        let offer = run(updater.check()).unwrap().expect("an update");
        assert_eq!(offer, UpdateOffer { version: "9.9.9".into(), notes: Some("- something new".into()) });

        let seen = Arc::new(Mutex::new(None));
        let sink = seen.clone();
        run(updater.download(move |p| *sink.lock().unwrap() = Some(p))).unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(Progress { downloaded: PAYLOAD.len() as u64, total: Some(PAYLOAD.len() as u64) }));
        assert_eq!(updater.staged.lock().unwrap().bytes.as_deref(), Some(PAYLOAD));
    }

    #[test]
    fn current_and_older_releases_are_not_offered() {
        let keys = sign(PAYLOAD);
        let signature = keys.signature.clone();
        let base = serve(move |base| manifest("0.1.0", base, &signature), PAYLOAD.to_vec());
        let updater = updater_for(&base, &keys.public);
        assert_eq!(run(updater.check()).unwrap(), None);
    }

    #[test]
    fn a_tampered_download_is_refused() {
        let keys = sign(PAYLOAD);
        let signature = keys.signature.clone();
        let base = serve(move |base| manifest("9.9.9", base, &signature), b"something else entirely".to_vec());
        let updater = updater_for(&base, &keys.public);
        run(updater.check()).unwrap().expect("an update");
        assert!(run(updater.download(|_| {})).is_err());
        assert!(updater.staged.lock().unwrap().bytes.is_none());
    }

    #[test]
    fn a_release_signed_by_another_key_is_refused() {
        let attacker = sign(PAYLOAD);
        let ours = sign(b"unrelated");
        let signature = attacker.signature.clone();
        let base = serve(move |base| manifest("9.9.9", base, &signature), PAYLOAD.to_vec());
        let updater = updater_for(&base, &ours.public);
        run(updater.check()).unwrap().expect("an update");
        assert!(run(updater.download(|_| {})).is_err());
    }

    #[test]
    fn downloading_needs_a_check_first() {
        let updater = Updater::with_feed("http://127.0.0.1:9/latest.json".parse().unwrap(), "", Version::new(0, 1, 0));
        assert!(run(updater.download(|_| {})).is_err());
    }

    #[test]
    fn a_build_without_a_key_reports_why() {
        // The test build has no HEARTH_UPDATER_PUBKEY (and is a debug build).
        let updater = Updater::new();
        assert!(updater.unavailable_reason().is_some());
        assert!(run(updater.check()).is_err());
    }

    #[test]
    fn versions_parse_with_or_without_a_v() {
        assert_eq!(parse_version("v1.2.3"), Some(Version::new(1, 2, 3)));
        assert_eq!(parse_version("0.1.0"), Some(Version::new(0, 1, 0)));
        assert_eq!(parse_version("nope"), None);
    }
}
