//! Server passwords live in the OS keychain, not in config.json. When no
//! keychain is available (e.g. Linux without a Secret Service), they fall back
//! to the config file, which is written with 0600 permissions.

use crate::config::ServerConfig;

const SERVICE: &str = "app.hearth.irc";

fn entry(server: &str, kind: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("{server}:{kind}")).ok()
}

fn get(server: &str, kind: &str) -> Option<String> {
    match entry(server, kind)?.get_password() {
        Ok(v) => Some(v),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            log::warn!("keychain read failed: {e}");
            None
        }
    }
}

/// Store (or with an empty value, remove) one secret. Returns false when the
/// keychain is unavailable.
fn put(server: &str, kind: &str, value: &str) -> bool {
    let Some(e) = entry(server, kind) else { return false };
    let res = if value.is_empty() {
        match e.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        e.set_password(value)
    };
    match res {
        Ok(()) => true,
        Err(e) => {
            log::warn!("keychain write failed, keeping secrets in the config file: {e}");
            false
        }
    }
}

/// Move a server's secrets into the keychain. Sets `in_keychain` on success.
pub fn store(server: &mut ServerConfig) {
    server.in_keychain = put(&server.id, "password", &server.password) && put(&server.id, "sasl", &server.sasl_password);
}

/// Fill in secrets for a server loaded from disk. Returns true when secrets
/// were found in the file and migrated, so the caller should re-save.
pub fn load(server: &mut ServerConfig) -> bool {
    if !server.password.is_empty() || !server.sasl_password.is_empty() {
        store(server);
        return server.in_keychain;
    }
    let password = get(&server.id, "password");
    let sasl = get(&server.id, "sasl");
    if password.is_some() || sasl.is_some() {
        server.password = password.unwrap_or_default();
        server.sasl_password = sasl.unwrap_or_default();
        server.in_keychain = true;
    }
    false
}

pub fn forget(server: &str) {
    put(server, "password", "");
    put(server, "sasl", "");
}
