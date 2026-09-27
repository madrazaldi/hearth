//! Persistent configuration: server definitions plus opaque UI settings.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub servers: Vec<ServerConfig>,
    /// Owned by the frontend; stored verbatim.
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SaslMechanism {
    #[default]
    None,
    /// Account and password: SCRAM-SHA-256 when offered, else PLAIN.
    Plain,
    /// A TLS client certificate (CertFP).
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub accept_invalid_certs: bool,
    pub websocket: bool,
    pub websocket_path: String,
    pub nick: String,
    pub username: String,
    pub realname: String,
    /// Server password (PASS).
    pub password: String,
    pub sasl: SaslMechanism,
    pub sasl_username: String,
    pub sasl_password: String,
    pub autoconnect: bool,
    pub autojoin: Vec<String>,
    /// Raw lines sent after registration.
    pub perform: Vec<String>,
    /// PEM file holding a client certificate and its private key.
    pub client_cert: String,
    /// Secrets are held in the OS keychain rather than this file.
    #[serde(skip)]
    pub in_keychain: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            id: String::new(),
            name: String::new(),
            host: String::new(),
            port: 6697,
            tls: true,
            accept_invalid_certs: false,
            websocket: false,
            websocket_path: "/".into(),
            nick: String::new(),
            username: String::new(),
            realname: String::new(),
            password: String::new(),
            sasl: SaslMechanism::None,
            sasl_username: String::new(),
            sasl_password: String::new(),
            autoconnect: true,
            autojoin: Vec::new(),
            perform: Vec::new(),
            client_cert: String::new(),
            in_keychain: false,
        }
    }
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Store { path: dir.join("config.json") }
    }

    /// Load the config and fill in secrets from the keychain, migrating any
    /// that are still stored in the file.
    pub fn load(&self) -> Config {
        let mut config = self.read();
        let mut migrated = false;
        for s in &mut config.servers {
            migrated |= crate::secrets::load(s);
        }
        if migrated {
            if let Err(e) = self.save(&config) {
                log::error!("could not rewrite config after moving secrets to the keychain: {e}");
            }
        }
        config
    }

    fn read(&self) -> Config {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::error!("config at {} is invalid ({e}); starting fresh", self.path.display());
                let _ = std::fs::copy(&self.path, self.path.with_extension("json.bak"));
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut on_disk = config.clone();
        for s in on_disk.servers.iter_mut().filter(|s| s.in_keychain) {
            s.password.clear();
            s.sasl_password.clear();
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&on_disk)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path).context("writing config")?;
        Ok(())
    }
}
