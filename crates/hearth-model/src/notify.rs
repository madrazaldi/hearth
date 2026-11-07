//! When a message deserves a system notification, and how it reads.

use crate::effect::Effect;
use crate::types::{NotifyLevel, Settings};

/// `level` is the buffer's own setting (`None` means "default"); `viewing` is
/// true while the window is focused on that buffer.
pub fn should_notify(settings: &Settings, level: Option<NotifyLevel>, is_query: bool, important: bool, viewing: bool) -> bool {
    if level == Some(NotifyLevel::None) || (!important && level != Some(NotifyLevel::All)) {
        return false;
    }
    if level.is_none() && (if is_query { !settings.notify_private } else { !settings.notify_highlights }) {
        return false;
    }
    !viewing
}

/// The notification effect, with the body cut to a size the OS shows.
pub fn notification(title: &str, body: &str) -> Effect {
    let body = if body.chars().count() > 240 { body.chars().take(239).collect::<String>() + "\u{2026}" } else { body.to_string() };
    Effect::Notify { title: title.to_string(), body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_buffer_level_then_the_defaults() {
        let mut s = Settings::default();
        assert!(should_notify(&s, None, false, true, false));
        assert!(!should_notify(&s, None, false, false, false));
        assert!(should_notify(&s, Some(NotifyLevel::All), false, false, false));
        assert!(!should_notify(&s, Some(NotifyLevel::None), true, true, false));
        assert!(!should_notify(&s, None, true, true, true));
        s.notify_private = false;
        assert!(!should_notify(&s, None, true, true, false));
        assert!(should_notify(&s, Some(NotifyLevel::Mentions), true, true, false));
    }
}
