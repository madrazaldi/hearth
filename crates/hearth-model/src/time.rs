//! Clock and date formatting. Everything takes a [`Zone`] so tests can pin
//! the offset instead of depending on the machine's timezone.

use crate::types::{Clock, Settings};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Zone {
    /// The machine's timezone.
    #[default]
    Local,
    /// A fixed offset in seconds east of UTC.
    Fixed(i32),
}

impl Zone {
    pub const UTC: Zone = Zone::Fixed(0);

    /// `ms` as a calendar date and time of day in this zone.
    pub fn naive(self, ms: i64) -> NaiveDateTime {
        let utc = Utc.timestamp_millis_opt(ms).single().unwrap_or_default();
        match self {
            Zone::Local => utc.with_timezone(&Local).naive_local(),
            Zone::Fixed(secs) => utc.with_timezone(&FixedOffset::east_opt(secs).unwrap_or(FixedOffset::east_opt(0).unwrap())).naive_local(),
        }
    }

    /// Epoch milliseconds of a local date and time.
    pub fn millis(self, naive: NaiveDateTime) -> i64 {
        match self {
            Zone::Local => {
                let t = Local.from_local_datetime(&naive);
                // A time skipped by a DST jump: take the same wall time an hour later.
                t.earliest()
                    .or_else(|| Local.from_local_datetime(&(naive + Duration::hours(1))).earliest())
                    .map(|d| d.timestamp_millis())
                    .unwrap_or_else(|| naive.and_utc().timestamp_millis())
            }
            Zone::Fixed(secs) => naive.and_utc().timestamp_millis() - secs as i64 * 1000,
        }
    }

    pub fn date(self, ms: i64) -> NaiveDate {
        self.naive(ms).date()
    }

    /// Local midnight at the start of `date`.
    pub fn midnight(self, date: NaiveDate) -> i64 {
        self.millis(date.and_hms_opt(0, 0, 0).unwrap())
    }

    /// The start and end (exclusive) of the local day containing `ms`.
    pub fn day_bounds(self, ms: i64) -> (i64, i64) {
        let d = self.date(ms);
        (self.midnight(d), self.midnight(d + Duration::days(1)))
    }
}

pub fn format_clock(ms: i64, clock: Clock, show_seconds: bool, zone: Zone) -> String {
    let t = zone.naive(ms);
    let sec = if show_seconds { format!(":{:02}", t.second()) } else { String::new() };
    match clock {
        Clock::H12 => {
            let ap = if t.hour() >= 12 { 'p' } else { 'a' };
            let h = match t.hour() % 12 {
                0 => 12,
                h => h,
            };
            format!("{h}:{:02}{sec}{ap}", t.minute())
        }
        Clock::H24 => format!("{:02}:{:02}{sec}", t.hour(), t.minute()),
    }
}

pub fn format_clock_for(ms: i64, s: &Settings, zone: Zone) -> String {
    format_clock(ms, s.clock, s.show_seconds, zone)
}

pub fn day_key(ms: i64, zone: Zone) -> (i32, u32, u32) {
    let d = zone.date(ms);
    (d.year(), d.month0(), d.day())
}

/// "Today", "Yesterday", or a long date like "Thursday, November 5".
pub fn format_day(ms: i64, now: i64, zone: Zone) -> String {
    if day_key(ms, zone) == day_key(now, zone) {
        return "Today".into();
    }
    if day_key(ms, zone) == day_key(now - 86_400_000, zone) {
        return "Yesterday".into();
    }
    let d = zone.date(ms);
    let base = d.format("%A, %B %-d").to_string();
    if d.year() == zone.date(now).year() {
        base
    } else {
        format!("{base}, {}", d.year())
    }
}

/// "Nov 5, 2026, 3:04:05 PM"
pub fn format_full(ms: i64, zone: Zone) -> String {
    zone.naive(ms).format("%b %-d, %Y, %-I:%M:%S %p").to_string()
}

/// "Nov 5, 2026, 3:04 PM"
pub fn format_short(ms: i64, zone: Zone) -> String {
    zone.naive(ms).format("%b %-d, %Y, %-I:%M %p").to_string()
}

pub fn format_duration(mut secs: u64) -> String {
    let units = [("d", 86_400u64), ("h", 3600), ("m", 60), ("s", 1)];
    let mut parts: Vec<String> = Vec::new();
    for (u, n) in units {
        if secs >= n {
            parts.push(format!("{}{u}", secs / n));
            secs %= n;
        }
        if parts.len() == 2 {
            break;
        }
    }
    if parts.is_empty() {
        "0s".into()
    } else {
        parts.join(" ")
    }
}

/// Epoch milliseconds now, from the system clock.
pub fn system_now() -> i64 {
    DateTime::<Utc>::from(std::time::SystemTime::now()).timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z: Zone = Zone::UTC;
    // 2026-11-05 15:04:05 UTC
    const T: i64 = 1_793_891_045_000;

    #[test]
    fn formats_clocks() {
        assert_eq!(format_clock(T, Clock::H24, false, Z), "15:04");
        assert_eq!(format_clock(T, Clock::H24, true, Z), "15:04:05");
        assert_eq!(format_clock(T, Clock::H12, false, Z), "3:04p");
        assert_eq!(format_clock(T - 15 * 3_600_000, Clock::H12, true, Z), "12:04:05a");
    }

    #[test]
    fn formats_days_and_durations() {
        assert_eq!(format_day(T, T, Z), "Today");
        assert_eq!(format_day(T - 86_400_000, T, Z), "Yesterday");
        assert_eq!(format_day(T - 3 * 86_400_000, T, Z), "Monday, November 2");
        assert_eq!(format_day(T - 400 * 86_400_000, T, Z), "Wednesday, October 1, 2025");
        assert_eq!(format_full(T, Z), "Nov 5, 2026, 3:04:05 PM");
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(3725), "1h 2m");
        assert_eq!(format_duration(90_000), "1d 1h");
    }

    #[test]
    fn day_bounds_follow_the_zone() {
        let (a, b) = Zone::Fixed(3600).day_bounds(T);
        assert_eq!(b - a, 86_400_000);
        assert_eq!(Zone::Fixed(3600).naive(a).hour(), 0);
    }
}
