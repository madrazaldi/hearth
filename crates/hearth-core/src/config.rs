//! Persistent configuration: server definitions plus opaque UI settings.

use crate::secrets::{self, Keychain};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub servers: Vec<ServerConfig>,
    /// Owned by the app; stored verbatim. Always a JSON object.
    pub settings: Value,
}

impl Default for Config {
    fn default() -> Self {
        Config { servers: Vec::new(), settings: Value::Object(Map::new()) }
    }
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
    keychain: Arc<dyn Keychain>,
}

impl Store {
    pub fn new(dir: &Path, keychain: Arc<dyn Keychain>) -> Self {
        Store { path: dir.join("config.json"), keychain }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the config and fill in secrets from the keychain, migrating any
    /// that are still stored in the file.
    pub fn load(&self) -> Config {
        let mut config = self.read();
        let mut migrated = false;
        for s in &mut config.servers {
            migrated |= secrets::load(self.keychain.as_ref(), s);
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
            Ok(s) => parse(&s).unwrap_or_else(|e| {
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
        write_private(&tmp, &serde_json::to_vec_pretty(&on_disk)?)?;
        std::fs::rename(&tmp, &self.path).context("writing config")?;
        Ok(())
    }
}

/// For files that hold secrets: created readable by their owner only, rather
/// than restricted afterwards.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    #[cfg(unix)]
    {
        // A leftover file from an older build keeps its old mode otherwise.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

/// Parse a config file. A null where a value is expected means "use the
/// default", as a missing field does; any other wrong type is an error.
fn parse(text: &str) -> Result<Config, serde_json::Error> {
    let mut raw: Value = serde_json::from_str(text)?;
    let Some(top) = raw.as_object_mut() else {
        return Err(serde::de::Error::custom("not an object"));
    };
    top.retain(|_, v| !v.is_null());
    // Serde would read a JSON array as a struct's fields in order.
    if raw.get("servers").and_then(Value::as_array).is_some_and(|l| l.iter().any(|s| !s.is_object())) {
        return Err(serde::de::Error::custom("server must be an object"));
    }
    if let Some(servers) = raw.get_mut("servers").and_then(Value::as_array_mut) {
        for server in servers.iter_mut().filter_map(Value::as_object_mut) {
            server.retain(|_, v| !v.is_null());
        }
    }
    let mut config: Config = serde_json::from_value(raw)?;
    if !config.settings.is_object() {
        config.settings = Value::Object(Map::new());
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryKeychain;
    use crate::test_util::TempDir;
    use serde_json::json;

    fn store(dir: &TempDir) -> (Store, Arc<MemoryKeychain>) {
        let vault = Arc::new(MemoryKeychain::default());
        (Store::new(dir.path(), vault.clone()), vault)
    }

    fn write(dir: &TempDir, value: Value) {
        std::fs::write(dir.path().join("config.json"), value.to_string()).unwrap();
    }

    fn file(dir: &TempDir) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("config.json")).unwrap()).unwrap()
    }

    /// A keychain that is never available.
    struct Broken;

    impl Keychain for Broken {
        fn get(&self, _: &str, _: &str) -> Result<Option<String>, String> {
            Err("no keychain".into())
        }
        fn set(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Err("no keychain".into())
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), String> {
            Err("no keychain".into())
        }
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_config() {
        let dir = TempDir::new();
        let config = store(&dir).0.load();
        assert!(config.servers.is_empty());
        assert_eq!(config.settings, json!({}));
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let dir = TempDir::new();
        write(&dir, json!({"servers": [{"id": "a", "host": "irc.example", "nick": "me"}]}));
        let s = store(&dir).0.load().servers.remove(0);
        assert_eq!((s.id.as_str(), s.host.as_str(), s.nick.as_str()), ("a", "irc.example", "me"));
        assert_eq!((s.port, s.tls, s.autoconnect, s.websocket_path.as_str()), (6697, true, true, "/"));
        assert_eq!(s.sasl, SaslMechanism::None);
        // A null counts as missing.
        write(&dir, json!({"servers": [{"id": "a", "port": null, "tls": null}], "settings": null}));
        let c = store(&dir).0.load();
        assert_eq!((c.servers[0].port, c.servers[0].tls), (6697, true));
        assert_eq!(c.settings, json!({}));
    }

    #[test]
    fn wrongly_typed_fields_are_rejected() {
        for bad in [json!({"port": "6667"}), json!({"port": 70000}), json!({"tls": "yes"}), json!({"autojoin": [1]}), json!({"sasl": "scram"}), json!([])] {
            assert!(parse(&json!({"servers": [bad.clone()]}).to_string()).is_err(), "{bad}");
        }
        assert!(parse("[]").is_err());
        assert!(parse(&json!({"servers": {}}).to_string()).is_err());
    }

    #[test]
    fn the_layout_matches_what_earlier_builds_wrote() {
        let dir = TempDir::new();
        write(
            &dir,
            json!({
                "servers": [{
                    "id": "abc123", "name": "Libera", "host": "irc.libera.chat", "port": 6697, "tls": true,
                    "acceptInvalidCerts": false, "websocket": false, "websocketPath": "/", "nick": "me",
                    "username": "", "realname": "", "password": "", "sasl": "plain", "saslUsername": "acct",
                    "saslPassword": "", "autoconnect": true, "autojoin": ["#a"], "perform": [], "clientCert": ""
                }],
                "settings": {"theme": "dark"}
            }),
        );
        let c = store(&dir).0.load();
        assert_eq!((c.servers[0].sasl_username.as_str(), c.servers[0].sasl.clone()), ("acct", SaslMechanism::Plain));
        assert_eq!(c.settings, json!({"theme": "dark"}));
        // And what is written has the same keys, with none of ours.
        let (store, _) = store(&dir);
        store.save(&c).unwrap();
        let written = file(&dir);
        let mut keys: Vec<_> = written["servers"][0].as_object().unwrap().keys().cloned().collect();
        keys.sort();
        let mut want: Vec<_> = [
            "id",
            "name",
            "host",
            "port",
            "tls",
            "acceptInvalidCerts",
            "websocket",
            "websocketPath",
            "nick",
            "username",
            "realname",
            "password",
            "sasl",
            "saslUsername",
            "saslPassword",
            "autoconnect",
            "autojoin",
            "perform",
            "clientCert",
        ]
        .map(String::from)
        .to_vec();
        want.sort();
        assert_eq!(keys, want);
    }

    #[test]
    fn save_writes_atomically_with_private_permissions_and_no_internal_fields() {
        let dir = TempDir::new();
        let (store, _) = store(&dir);
        let s = ServerConfig { id: "a".into(), host: "h".into(), nick: "n".into(), password: "pw".into(), ..Default::default() };
        store.save(&Config { servers: vec![s], settings: json!({"x": 1}) }).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        assert!(!text.contains("inKeychain"));
        assert_eq!(file(&dir)["servers"][0]["password"], "pw", "no keychain: stays in the file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.path().join("config.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(!dir.path().join("config.json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_looser_file_from_an_older_build_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let (store, _) = store(&dir);
        let tmp = dir.path().join("config.json.tmp");
        std::fs::write(&tmp, "old").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        store.save(&Config::default()).unwrap();
        assert_eq!(std::fs::metadata(dir.path().join("config.json")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn passwords_in_the_file_move_to_the_keychain_on_load() {
        let dir = TempDir::new();
        write(&dir, json!({"servers": [{"id": "a", "host": "h", "nick": "n", "password": "pw", "saslPassword": "sp"}], "settings": {}}));
        let (store, vault) = store(&dir);
        let c = store.load();
        assert_eq!(c.servers[0].password, "pw", "still usable in memory");
        assert_eq!(vault.get(secrets::SERVICE, "a:password").unwrap().as_deref(), Some("pw"));
        assert_eq!(vault.get(secrets::SERVICE, "a:sasl").unwrap().as_deref(), Some("sp"));
        let f = file(&dir);
        assert_eq!((f["servers"][0]["password"].as_str(), f["servers"][0]["saslPassword"].as_str()), (Some(""), Some("")));
        // A fresh load reads them back from the keychain.
        let again = Store::new(dir.path(), vault).load();
        assert_eq!((again.servers[0].password.as_str(), again.servers[0].sasl_password.as_str()), ("pw", "sp"));
    }

    #[test]
    fn without_a_keychain_the_secrets_stay_in_the_file() {
        let dir = TempDir::new();
        write(&dir, json!({"servers": [{"id": "a", "host": "h", "nick": "n", "password": "pw"}], "settings": {}}));
        let c = Store::new(dir.path(), Arc::new(Broken)).load();
        assert_eq!(c.servers[0].password, "pw");
        assert_eq!(file(&dir)["servers"][0]["password"], "pw");
    }

    #[test]
    fn an_invalid_file_is_backed_up_and_replaced_by_an_empty_config() {
        let dir = TempDir::new();
        std::fs::write(dir.path().join("config.json"), "{ not json").unwrap();
        let c = store(&dir).0.load();
        assert!(c.servers.is_empty());
        assert_eq!(std::fs::read_to_string(dir.path().join("config.json.bak")).unwrap(), "{ not json");
    }

    #[test]
    fn settings_that_are_not_an_object_become_one() {
        let dir = TempDir::new();
        write(&dir, json!({"servers": [], "settings": [1, 2]}));
        assert_eq!(store(&dir).0.load().settings, json!({}));
    }
}
