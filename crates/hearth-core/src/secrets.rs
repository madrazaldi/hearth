//! Server passwords live in the OS keychain, not in config.json. When no
//! keychain is available (e.g. Linux without a Secret Service), they fall back
//! to the config file, which is written with 0600 permissions.
//!
//! The entries are the ones the Electrobun build kept with `Bun.secrets`, so
//! saved passwords carry over between the two apps. Its naming, per platform:
//!
//! - macOS: a generic password with service `app.hearth.irc` and account
//!   `<server id>:<kind>`, which is what the `keyring` crate writes too.
//! - Windows: a generic credential with target `service/account`, user name
//!   `account` and the UTF-8 bytes of the secret as its blob.
//! - Linux: a libsecret item of schema `com.oven-sh.bun.Secret` with the
//!   attributes `service` and `account`.
//!
//! The Rust engine before it used the `keyring` crate's own layout on Windows
//! (target `account.service`, UTF-16 blob) and Linux (attributes `service`
//! and `username`). Those entries are still looked up, copied into the format
//! above and removed, so a password cleared later stays cleared.

use crate::config::ServerConfig;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SERVICE: &str = "app.hearth.irc";

/// The keychain operations Hearth needs. Tests use [`MemoryKeychain`]; they
/// must never touch the real one.
pub trait Keychain: Send + Sync {
    fn get(&self, service: &str, name: &str) -> Result<Option<String>, String>;
    fn set(&self, service: &str, name: &str, value: &str) -> Result<(), String>;
    /// Removing an entry that does not exist is not an error.
    fn delete(&self, service: &str, name: &str) -> Result<(), String>;
    /// An entry the earlier Rust engine wrote, on platforms where its format
    /// differs from the one above.
    fn legacy_get(&self, _service: &str, _name: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn legacy_delete(&self, _service: &str, _name: &str) -> Result<(), String> {
        Ok(())
    }
}

/// The operating system's keychain.
pub fn system() -> Arc<dyn Keychain> {
    Arc::new(SystemKeychain)
}

struct SystemKeychain;

impl Keychain for SystemKeychain {
    fn get(&self, service: &str, name: &str) -> Result<Option<String>, String> {
        platform::get(service, name)
    }
    fn set(&self, service: &str, name: &str, value: &str) -> Result<(), String> {
        platform::set(service, name, value)
    }
    fn delete(&self, service: &str, name: &str) -> Result<(), String> {
        platform::delete(service, name)
    }
    fn legacy_get(&self, service: &str, name: &str) -> Result<Option<String>, String> {
        platform::legacy_get(service, name)
    }
    fn legacy_delete(&self, service: &str, name: &str) -> Result<(), String> {
        platform::legacy_delete(service, name)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::keyring_result;

    fn entry(service: &str, name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(service, name).map_err(|e| e.to_string())
    }

    pub fn get(service: &str, name: &str) -> Result<Option<String>, String> {
        keyring_result(entry(service, name)?.get_password())
    }

    pub fn set(service: &str, name: &str, value: &str) -> Result<(), String> {
        entry(service, name)?.set_password(value).map_err(|e| e.to_string())
    }

    pub fn delete(service: &str, name: &str) -> Result<(), String> {
        keyring_result(entry(service, name)?.delete_credential()).map(|_| ())
    }

    // The keyring crate wrote the same entries Bun reads.
    pub fn legacy_get(_: &str, _: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    pub fn legacy_delete(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::keyring_result;

    fn entry(service: &str, name: &str) -> Result<keyring::Entry, String> {
        // Windows finds a credential by its target name alone.
        keyring::Entry::new_with_target(&format!("{service}/{name}"), service, name).map_err(|e| e.to_string())
    }

    pub fn get(service: &str, name: &str) -> Result<Option<String>, String> {
        // The blob is raw UTF-8; `get_password` would read it as UTF-16.
        Ok(keyring_result(entry(service, name)?.get_secret())?.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    pub fn set(service: &str, name: &str, value: &str) -> Result<(), String> {
        entry(service, name)?.set_secret(value.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn delete(service: &str, name: &str) -> Result<(), String> {
        keyring_result(entry(service, name)?.delete_credential()).map(|_| ())
    }

    fn old(service: &str, name: &str) -> Result<keyring::Entry, String> {
        // target `name.service`, UTF-16 blob
        keyring::Entry::new(service, name).map_err(|e| e.to_string())
    }

    pub fn legacy_get(service: &str, name: &str) -> Result<Option<String>, String> {
        keyring_result(old(service, name)?.get_password())
    }

    pub fn legacy_delete(service: &str, name: &str) -> Result<(), String> {
        keyring_result(old(service, name)?.delete_credential()).map(|_| ())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{keyring_result, libsecret};

    pub fn get(service: &str, name: &str) -> Result<Option<String>, String> {
        libsecret::lookup(service, name)
    }

    pub fn set(service: &str, name: &str, value: &str) -> Result<(), String> {
        libsecret::store(service, name, value)
    }

    pub fn delete(service: &str, name: &str) -> Result<(), String> {
        libsecret::clear(service, name)
    }

    fn old(service: &str, name: &str) -> Result<keyring::Entry, String> {
        // attributes `service` and `username`
        keyring::Entry::new(service, name).map_err(|e| e.to_string())
    }

    pub fn legacy_get(service: &str, name: &str) -> Result<Option<String>, String> {
        keyring_result(old(service, name)?.get_password())
    }

    pub fn legacy_delete(service: &str, name: &str) -> Result<(), String> {
        keyring_result(old(service, name)?.delete_credential()).map(|_| ())
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    const NONE: &str = "no keychain on this platform";

    pub fn get(_: &str, _: &str) -> Result<Option<String>, String> {
        Err(NONE.into())
    }
    pub fn set(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err(NONE.into())
    }
    pub fn delete(_: &str, _: &str) -> Result<(), String> {
        Err(NONE.into())
    }
    pub fn legacy_get(_: &str, _: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    pub fn legacy_delete(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A missing entry is an absent value, not a failure.
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_result<T>(r: keyring::Result<T>) -> Result<Option<T>, String> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

// libsecret is reached the way Bun does, so both apps write identical items.
#[cfg(any(target_os = "linux", all(test, unix)))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod libsecret;

/// An in-memory keychain, for tests and for hosts that want none.
#[derive(Default)]
pub struct MemoryKeychain {
    vault: Mutex<HashMap<String, String>>,
    legacy: Mutex<HashMap<String, String>>,
}

impl MemoryKeychain {
    fn key(service: &str, name: &str) -> String {
        format!("{service}/{name}")
    }

    /// Plant an entry in the old layout, as an earlier engine would have left it.
    pub fn put_legacy(&self, service: &str, name: &str, value: &str) {
        self.legacy.lock().unwrap().insert(Self::key(service, name), value.into());
    }

    pub fn len(&self) -> usize {
        self.vault.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn legacy_len(&self) -> usize {
        self.legacy.lock().unwrap().len()
    }

    /// Move everything in the current layout to the old one, as if another
    /// engine had stored it.
    pub fn demote_all(&self) {
        let moved = std::mem::take(&mut *self.vault.lock().unwrap());
        self.legacy.lock().unwrap().extend(moved);
    }
}

impl Keychain for MemoryKeychain {
    fn get(&self, service: &str, name: &str) -> Result<Option<String>, String> {
        Ok(self.vault.lock().unwrap().get(&Self::key(service, name)).cloned())
    }
    fn set(&self, service: &str, name: &str, value: &str) -> Result<(), String> {
        self.vault.lock().unwrap().insert(Self::key(service, name), value.into());
        Ok(())
    }
    fn delete(&self, service: &str, name: &str) -> Result<(), String> {
        self.vault.lock().unwrap().remove(&Self::key(service, name));
        Ok(())
    }
    fn legacy_get(&self, service: &str, name: &str) -> Result<Option<String>, String> {
        Ok(self.legacy.lock().unwrap().get(&Self::key(service, name)).cloned())
    }
    fn legacy_delete(&self, service: &str, name: &str) -> Result<(), String> {
        self.legacy.lock().unwrap().remove(&Self::key(service, name));
        Ok(())
    }
}

fn account(server: &str, kind: &str) -> String {
    format!("{server}:{kind}")
}

fn get(kc: &dyn Keychain, server: &str, kind: &str) -> Option<String> {
    let name = account(server, kind);
    match kc.get(SERVICE, &name) {
        Ok(Some(v)) => return Some(v),
        Ok(None) => {}
        Err(e) => {
            log::warn!("keychain read failed: {e}");
            return None;
        }
    }
    let value = match kc.legacy_get(SERVICE, &name) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("legacy keychain read failed: {e}");
            return None;
        }
    };
    if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
        // Left behind, the old entry would bring the secret back after the user clears it.
        if let Err(e) = kc.set(SERVICE, &name, v).and_then(|()| kc.legacy_delete(SERVICE, &name)) {
            log::warn!("could not migrate a keychain entry: {e}");
        }
    }
    value
}

/// Store (or with an empty value, remove) one secret. Returns false when the
/// keychain is unavailable.
fn put(kc: &dyn Keychain, server: &str, kind: &str, value: &str) -> bool {
    let name = account(server, kind);
    let res = if value.is_empty() { kc.delete(SERVICE, &name).and_then(|()| kc.legacy_delete(SERVICE, &name)) } else { kc.set(SERVICE, &name, value) };
    match res {
        Ok(()) => true,
        Err(e) => {
            log::warn!("keychain write failed, keeping secrets in the config file: {e}");
            false
        }
    }
}

/// Move a server's secrets into the keychain. Sets `in_keychain` on success.
pub fn store(kc: &dyn Keychain, server: &mut ServerConfig) {
    server.in_keychain = put(kc, &server.id, "password", &server.password) && put(kc, &server.id, "sasl", &server.sasl_password);
}

/// Fill in secrets for a server loaded from disk. Returns true when secrets
/// were found in the file and migrated, so the caller should re-save.
pub fn load(kc: &dyn Keychain, server: &mut ServerConfig) -> bool {
    if !server.password.is_empty() || !server.sasl_password.is_empty() {
        store(kc, server);
        return server.in_keychain;
    }
    let password = get(kc, &server.id, "password");
    let sasl = get(kc, &server.id, "sasl");
    if password.is_some() || sasl.is_some() {
        server.password = password.unwrap_or_default();
        server.sasl_password = sasl.unwrap_or_default();
        server.in_keychain = true;
    }
    false
}

pub fn forget(kc: &dyn Keychain, server: &str) {
    put(kc, server, "password", "");
    put(kc, server, "sasl", "");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(password: &str, sasl: &str) -> ServerConfig {
        ServerConfig { id: "x".into(), password: password.into(), sasl_password: sasl.into(), ..Default::default() }
    }

    /// A keychain that refuses everything, like a Linux box without a Secret Service.
    struct Unavailable;
    impl Keychain for Unavailable {
        fn get(&self, _: &str, _: &str) -> Result<Option<String>, String> {
            Err("no secret service".into())
        }
        fn set(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Err("no secret service".into())
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), String> {
            Err("no secret service".into())
        }
    }

    #[test]
    fn secrets_use_the_names_bun_reads() {
        let kc = MemoryKeychain::default();
        let mut s = server("pw", "sasl-pw");
        store(&kc, &mut s);
        assert!(s.in_keychain);
        assert_eq!(kc.get("app.hearth.irc", "x:password").unwrap().as_deref(), Some("pw"));
        assert_eq!(kc.get("app.hearth.irc", "x:sasl").unwrap().as_deref(), Some("sasl-pw"));
    }

    #[test]
    fn an_empty_secret_removes_the_entry() {
        let kc = MemoryKeychain::default();
        let mut s = server("pw", "");
        store(&kc, &mut s);
        assert_eq!(kc.len(), 1);
        s.password.clear();
        store(&kc, &mut s);
        assert!(kc.is_empty());
        assert!(s.in_keychain);
    }

    #[test]
    fn secrets_the_old_engine_stored_are_found_and_moved_over() {
        let kc = MemoryKeychain::default();
        kc.put_legacy(SERVICE, "x:password", "pw");
        let mut s = server("", "");
        assert!(!load(&kc, &mut s));
        assert_eq!(s.password, "pw");
        assert!(s.in_keychain);
        assert_eq!(kc.get(SERVICE, "x:password").unwrap().as_deref(), Some("pw"));
        // The old entry is gone, so clearing the password sticks.
        assert_eq!(kc.legacy_len(), 0);
        s.password.clear();
        store(&kc, &mut s);
        let mut again = server("", "");
        load(&kc, &mut again);
        assert_eq!(again.password, "");
        assert!(!again.in_keychain);
    }

    #[test]
    fn clearing_a_password_removes_a_leftover_old_entry_too() {
        let kc = MemoryKeychain::default();
        kc.put_legacy(SERVICE, "x:password", "stale");
        let mut s = server("", "");
        store(&kc, &mut s);
        assert_eq!(kc.legacy_len(), 0);
    }

    #[test]
    fn passwords_in_the_file_move_to_the_keychain() {
        let kc = MemoryKeychain::default();
        let mut s = server("pw", "");
        assert!(load(&kc, &mut s));
        assert!(s.in_keychain);
        assert_eq!(kc.len(), 1);
    }

    #[test]
    fn without_a_keychain_the_secrets_stay_in_the_file() {
        let mut s = server("pw", "");
        assert!(!load(&Unavailable, &mut s));
        assert!(!s.in_keychain);
        assert_eq!(s.password, "pw");
        // Reading a server whose secrets are not in the file finds nothing.
        let mut bare = server("", "");
        assert!(!load(&Unavailable, &mut bare));
        assert!(!bare.in_keychain);
    }

    #[test]
    fn forget_removes_both_entries() {
        let kc = MemoryKeychain::default();
        let mut s = server("pw", "sasl");
        store(&kc, &mut s);
        forget(&kc, "x");
        assert!(kc.is_empty());
    }
}
