//! Where Hearth keeps its files. The locations are the ones earlier builds
//! used (bundle identifier `app.hearth.irc`), so existing installs keep their
//! config and log.

use std::path::PathBuf;

pub const APP_ID: &str = "app.hearth.irc";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
}

/// The platforms whose layout differs; anything else follows XDG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Other,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Other
        }
    }
}

/// The config and data directories for this machine, honouring the
/// `HEARTH_CONFIG_DIR` and `HEARTH_DATA_DIR` overrides.
pub fn app_dirs() -> Dirs {
    let home = std::env::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    app_dirs_for(&|k| std::env::var(k).ok(), Platform::current(), &home)
}

/// `app_dirs` with the environment, platform and home directory passed in, so
/// every branch can be tested anywhere.
pub fn app_dirs_for(env: &dyn Fn(&str) -> Option<String>, platform: Platform, home: &str) -> Dirs {
    // By the target platform's separator, not the host's.
    let sep = if platform == Platform::Windows { '\\' } else { '/' };
    let join = |base: &str, parts: &[&str]| {
        let mut out = base.trim_end_matches(sep).to_string();
        for p in parts {
            out.push(sep);
            out.push_str(p);
        }
        out
    };
    // An unset or empty variable counts as absent.
    let var = |k: &str| env(k).filter(|v| !v.is_empty());

    let (config, data) = match platform {
        Platform::MacOs => {
            let dir = join(home, &["Library", "Application Support", APP_ID]);
            (dir.clone(), dir)
        }
        Platform::Windows => {
            let base = var("APPDATA").unwrap_or_else(|| join(home, &["AppData", "Roaming"]));
            let dir = join(&base, &[APP_ID]);
            (dir.clone(), dir)
        }
        Platform::Other => {
            let config = var("XDG_CONFIG_HOME").unwrap_or_else(|| join(home, &[".config"]));
            let data = var("XDG_DATA_HOME").unwrap_or_else(|| join(home, &[".local", "share"]));
            (join(&config, &[APP_ID]), join(&data, &[APP_ID]))
        }
    };
    Dirs { config: var("HEARTH_CONFIG_DIR").unwrap_or(config).into(), data: var("HEARTH_DATA_DIR").unwrap_or(data).into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dirs(platform: Platform, home: &str, vars: &[(&str, &str)]) -> Dirs {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        app_dirs_for(&|k| vars.get(k).cloned(), platform, home)
    }

    #[test]
    fn macos_keeps_config_and_data_together_under_application_support() {
        let d = dirs(Platform::MacOs, "/Users/me", &[]);
        assert_eq!(d.config, PathBuf::from("/Users/me/Library/Application Support/app.hearth.irc"));
        assert_eq!(d.data, d.config);
    }

    #[test]
    fn linux_follows_the_xdg_directories() {
        let d = dirs(Platform::Other, "/home/me", &[]);
        assert_eq!(d.config, PathBuf::from("/home/me/.config/app.hearth.irc"));
        assert_eq!(d.data, PathBuf::from("/home/me/.local/share/app.hearth.irc"));
        let d = dirs(Platform::Other, "/home/me", &[("XDG_CONFIG_HOME", "/cfg"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(d.config, PathBuf::from("/cfg/app.hearth.irc"));
        assert_eq!(d.data, PathBuf::from("/data/app.hearth.irc"));
        // An empty variable is as good as unset.
        let d = dirs(Platform::Other, "/home/me", &[("XDG_CONFIG_HOME", "")]);
        assert_eq!(d.config, PathBuf::from("/home/me/.config/app.hearth.irc"));
    }

    #[test]
    fn windows_uses_roaming_app_data_with_backslashes() {
        let d = dirs(Platform::Windows, r"C:\Users\me", &[]);
        assert_eq!(d.config.to_str().unwrap(), r"C:\Users\me\AppData\Roaming\app.hearth.irc");
        assert_eq!(d.data, d.config);
        let d = dirs(Platform::Windows, r"C:\Users\me", &[("APPDATA", r"D:\Roaming")]);
        assert_eq!(d.config.to_str().unwrap(), r"D:\Roaming\app.hearth.irc");
    }

    #[test]
    fn the_override_variables_win() {
        let d = dirs(Platform::MacOs, "/Users/me", &[("HEARTH_CONFIG_DIR", "/tmp/c"), ("HEARTH_DATA_DIR", "/tmp/d")]);
        assert_eq!(d, Dirs { config: "/tmp/c".into(), data: "/tmp/d".into() });
        // Each is independent of the other.
        let d = dirs(Platform::Other, "/home/me", &[("HEARTH_DATA_DIR", "/tmp/d")]);
        assert_eq!(d.config, PathBuf::from("/home/me/.config/app.hearth.irc"));
        assert_eq!(d.data, PathBuf::from("/tmp/d"));
    }
}
