//! Keyboard shortcuts as data, for the help overlay and Settings, and the
//! actions the app-level handler dispatches. Keep in sync with the UI's key
//! bindings and the README.

/// Which modifier symbols to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Other,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Other
        }
    }

    /// The primary modifier: Command or Control.
    pub fn modifier(self) -> &'static str {
        match self {
            Platform::Mac => "\u{2318}",
            Platform::Other => "Ctrl",
        }
    }

    pub fn alt(self) -> &'static str {
        match self {
            Platform::Mac => "\u{2325}",
            Platform::Other => "Alt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// Alternative combos, each a list of keys pressed together.
    pub keys: Vec<Vec<String>>,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutGroup {
    pub title: &'static str,
    pub items: Vec<Shortcut>,
}

pub fn shortcut_groups(p: Platform) -> Vec<ShortcutGroup> {
    let (m, a) = (p.modifier(), p.alt());
    let combo = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let sc = |keys: &[&[&str]], label| Shortcut { keys: keys.iter().map(|k| combo(k)).collect(), label };
    vec![
        ShortcutGroup {
            title: "Navigation",
            items: vec![
                sc(&[&[m, "K"]], "Jump to a buffer"),
                sc(&[&[a, "\u{2191}"], &[a, "\u{2193}"]], "Previous / next buffer"),
                sc(&[&[a, "\u{21e7}", "\u{2191}"], &[a, "\u{21e7}", "\u{2193}"]], "Previous / next unread"),
                sc(&[&[m, "1\u{2013}9"]], "Go to channel by position"),
            ]
            .into_iter()
            // Cmd+W is the window menu's Close on macOS, so only the other platforms close buffers by key.
            .chain((p == Platform::Other).then(|| sc(&[&[m, "W"]], "Close buffer")))
            .collect(),
        },
        ShortcutGroup {
            title: "Panels",
            items: vec![
                sc(&[&[m, "F"]], "Filter messages"),
                sc(&[&[m, "\u{21e7}", "F"]], "Search all messages"),
                sc(&[&[m, "\u{21e7}", "M"]], "Toggle member list"),
                sc(&[&[m, ","]], "Settings"),
                sc(&[&[m, "/"]], "Keyboard shortcuts"),
                sc(&[&["Esc"]], "Close panel, cancel reply"),
            ],
        },
        ShortcutGroup {
            title: "Composer",
            items: vec![
                sc(&[&["Tab"], &["\u{21e7}", "Tab"]], "Complete nick, channel, command or alias"),
                sc(&[&[":", "name"]], "Suggest emoji (Tab or \u{21b5} picks, :name: inserts)"),
                sc(&[&["\u{2191}"], &["\u{2193}"]], "Input history"),
                sc(&[&["\u{21e7}", "\u{21b5}"]], "New line"),
            ],
        },
    ]
}

/// What an app-level shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Switcher,
    PrevBuffer,
    NextBuffer,
    PrevUnread,
    NextUnread,
    /// Go to the n-th channel (from 1) of the current network.
    NthChannel(usize),
    CloseBuffer,
    Filter,
    Search,
    ToggleMembers,
    Settings,
    Shortcuts,
    Escape,
}

/// The keys the app handler looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Command on macOS, Control elsewhere.
    pub primary: bool,
    pub alt: bool,
    pub shift: bool,
}

/// The action for a key press, if it is an app-level shortcut. `key` is the
/// lower-cased key name (`"k"`, `"arrowup"`, `"escape"`, `"1"`, `","`).
pub fn action_for(key: &str, m: Mods) -> Option<Action> {
    action_for_on(Platform::current(), key, m)
}

/// [`action_for`] on a given platform. Cmd+W is the window menu's Close on macOS, so only
/// Windows and Linux close the buffer with the primary key and W (as the help lists).
pub fn action_for_on(platform: Platform, key: &str, m: Mods) -> Option<Action> {
    if m.alt && !m.primary {
        return match (key, m.shift) {
            ("arrowup", false) => Some(Action::PrevBuffer),
            ("arrowdown", false) => Some(Action::NextBuffer),
            ("arrowup", true) => Some(Action::PrevUnread),
            ("arrowdown", true) => Some(Action::NextUnread),
            _ => None,
        };
    }
    if key == "escape" && !m.primary && !m.alt {
        return Some(Action::Escape);
    }
    if !m.primary || m.alt {
        return None;
    }
    match (key, m.shift) {
        ("k", false) => Some(Action::Switcher),
        ("w", false) if platform == Platform::Other => Some(Action::CloseBuffer),
        ("f", false) => Some(Action::Filter),
        ("f", true) => Some(Action::Search),
        ("m", true) => Some(Action::ToggleMembers),
        (",", false) => Some(Action::Settings),
        ("/", false) => Some(Action::Shortcuts),
        (d, false) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => Some(Action::NthChannel(d.parse().ok()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_follow_the_platform() {
        let mac = shortcut_groups(Platform::Mac);
        assert_eq!(mac[0].items[0].keys, vec![vec!["\u{2318}".to_string(), "K".to_string()]]);
        assert_eq!(shortcut_groups(Platform::Other)[0].items[1].keys[0][0], "Alt");
    }

    #[test]
    fn maps_keys_to_actions() {
        let p = Mods { primary: true, ..Mods::default() };
        assert_eq!(action_for("k", p), Some(Action::Switcher));
        assert_eq!(action_for("3", p), Some(Action::NthChannel(3)));
        assert_eq!(action_for("f", Mods { shift: true, ..p }), Some(Action::Search));
        assert_eq!(action_for("arrowdown", Mods { alt: true, shift: true, primary: false }), Some(Action::NextUnread));
        assert_eq!(action_for("k", Mods::default()), None);
        assert_eq!(action_for("escape", Mods::default()), Some(Action::Escape));
    }

    #[test]
    fn the_primary_w_closes_a_buffer_only_off_macos() {
        let p = Mods { primary: true, ..Mods::default() };
        assert_eq!(action_for_on(Platform::Other, "w", p), Some(Action::CloseBuffer));
        assert_eq!(action_for_on(Platform::Mac, "w", p), None, "Cmd+W closes the window through the menu");
        // The help text agrees: it lists Close buffer on Windows and Linux only.
        let lists = |pl| shortcut_groups(pl).iter().flat_map(|g| &g.items).any(|i| i.label == "Close buffer");
        assert!(lists(Platform::Other) && !lists(Platform::Mac));
    }
}
