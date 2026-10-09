//! Times and durations as the window says them: "2 min ago", "3 h 12 min", "14:32".

use chrono::{Local, TimeZone as _};

/// How long ago `at` was, from `now` (both Unix seconds): "just now", "4 min ago", "3 h ago", "yesterday",
/// "5 days ago", then the date.
#[must_use]
pub fn ago(now: i64, at: i64) -> String {
    let secs = (now - at).max(0);
    match secs {
        0..45 => "just now".to_owned(),
        45..3_600 => format!("{} min ago", ((secs + 30) / 60).max(1)),
        3_600..86_400 => format!("{} h ago", secs / 3_600),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..604_800 => format!("{} days ago", secs / 86_400),
        _ => date(at),
    }
}

/// A length of time, two units at most: "12 s", "1 min 20 s", "3 h 12 min", "2 days 4 h".
#[must_use]
pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m, s) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60, secs % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s} s"),
        (0, 0, _) if s == 0 => format!("{m} min"),
        (0, 0, _) => format!("{m} min {s} s"),
        (0, _, 0) => format!("{h} h"),
        (0, ..) => format!("{h} h {m} min"),
        (1, 0, _) => "1 day".to_owned(),
        (_, 0, _) => format!("{d} days"),
        (1, ..) => format!("1 day {h} h"),
        _ => format!("{d} days {h} h"),
    }
}

/// What is left of a pause, rounded up to the minute: "45 min", "3 h 12 min", "1 day 2 h".
#[must_use]
pub fn left(secs: i64) -> String {
    let minutes = (secs.max(0) + 59) / 60;
    duration(minutes * 60)
}

/// A setting's length in words: "1 hour", "15 minutes", "2 min 30 s".
#[must_use]
pub fn span(secs: u64) -> String {
    match secs {
        3_600 => "1 hour".to_owned(),
        86_400 => "1 day".to_owned(),
        60 => "1 minute".to_owned(),
        s if s % 3_600 == 0 => format!("{} hours", s / 3_600),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => duration(i64::try_from(s).unwrap_or(i64::MAX)),
    }
}

/// The local time of day: "14:32".
#[must_use]
pub fn clock(at: i64) -> String {
    Local.timestamp_opt(at, 0).single().map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// The local date: "Oct 3".
#[must_use]
pub fn date(at: i64) -> String {
    Local.timestamp_opt(at, 0).single().map_or_else(String::new, |t| t.format("%b %-d").to_string())
}

/// When, for the detail of an entry: "today 14:32:05", "Oct 3, 14:32:05".
#[must_use]
pub fn when(now: i64, at: i64) -> String {
    let Some(t) = Local.timestamp_opt(at, 0).single() else {
        return String::new();
    };
    let time = t.format("%H:%M:%S");
    if at >= start_of_day(now) {
        format!("today {time}")
    } else {
        format!("{}, {time}", t.format("%b %-d"))
    }
}

/// Unix seconds at the local midnight that began the day of `now`.
#[must_use]
pub fn start_of_day(now: i64) -> i64 {
    Local
        .timestamp_opt(now, 0)
        .single()
        .and_then(|t| t.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
        .map_or(now - now.rem_euclid(86_400), |t| t.timestamp())
}

/// `https://app.reins2fa.com` → `app.reins2fa.com`.
#[must_use]
pub fn host(server: &str) -> String {
    server.split("://").nth(1).unwrap_or(server).trim_end_matches('/').to_owned()
}

/// "1 request" / "3 requests".
#[must_use]
pub fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago_reads_like_speech() {
        assert_eq!(ago(1_000, 1_000), "just now");
        assert_eq!(ago(1_000, 1_100), "just now", "a clock a bit ahead");
        assert_eq!(ago(1_000, 950), "1 min ago");
        assert_eq!(ago(10_000, 10_000 - 125), "2 min ago");
        assert_eq!(ago(100_000, 100_000 - 3 * 3_600 - 5), "3 h ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 90_000), "yesterday");
        assert_eq!(ago(1_000_000, 1_000_000 - 3 * 86_400), "3 days ago");
        assert!(!ago(100_000_000, 1_000).contains("ago"));
    }

    #[test]
    fn durations_keep_two_units() {
        assert_eq!(duration(12), "12 s");
        assert_eq!(duration(60), "1 min");
        assert_eq!(duration(80), "1 min 20 s");
        assert_eq!(duration(3_600), "1 h");
        assert_eq!(duration(3 * 3_600 + 12 * 60 + 7), "3 h 12 min");
        assert_eq!(duration(86_400 + 60), "1 day");
        assert_eq!(duration(2 * 86_400 + 4 * 3_600), "2 days 4 h");
        assert_eq!(duration(-5), "0 s");
    }

    #[test]
    fn pauses_round_up_to_the_minute() {
        assert_eq!(left(1), "1 min");
        assert_eq!(left(59 * 60 + 1), "1 h");
        assert_eq!(left(3 * 3_600 + 11 * 60 + 30), "3 h 12 min");
        assert_eq!(left(0), "0 s");
    }

    #[test]
    fn spans_name_settings() {
        assert_eq!(span(3_600), "1 hour");
        assert_eq!(span(900), "15 minutes");
        assert_eq!(span(7_200), "2 hours");
        assert_eq!(span(150), "2 min 30 s");
    }

    #[test]
    fn midnight_is_before_now_and_within_a_day() {
        let now = 1_760_000_000;
        let midnight = start_of_day(now);
        assert!(midnight <= now && now - midnight < 26 * 3_600);
        assert!(when(now, now).starts_with("today "));
        assert!(!when(now, midnight - 10).starts_with("today"));
        assert_eq!(clock(now).len(), 5);
    }

    #[test]
    fn hosts_are_shown_without_the_scheme() {
        assert_eq!(host("https://app.reins2fa.com"), "app.reins2fa.com");
        assert_eq!(host("http://localhost:8000/"), "localhost:8000");
        assert_eq!(host("reins.example.org"), "reins.example.org");
        assert_eq!(count(1, "call", "calls"), "1 call");
        assert_eq!(count(4, "call", "calls"), "4 calls");
    }
}
