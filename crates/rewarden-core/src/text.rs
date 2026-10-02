//! Text hygiene for everything shown to the user or handed to an AI: bidi
//! controls are removed (a hostile subject must not visually reorder an
//! address), control characters are dropped, HTML is reduced to text.

/// Bidirectional-formatting characters (contracts: U+202A–U+202E, U+2066–U+2069,
/// plus the implicit marks LRM/RLM/ALM).
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Removes bidi controls and control characters; keeps `\n` and `\t`.
pub fn neutralize(s: &str) -> String {
    s.chars().filter(|c| !is_bidi_control(*c) && (!c.is_control() || matches!(c, '\n' | '\t'))).collect()
}

/// A single display line: neutralized, every whitespace/control run collapsed to one space.
pub fn one_line(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !is_bidi_control(*c))
        .map(|c| {
            if c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max_chars` characters, on a character boundary.
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// At most `max_bytes` bytes, cut on a character boundary.
pub fn truncate_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let num = entity.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// A rough HTML → text conversion: drops tags and `<script>`/`<style>` content,
/// turns block boundaries into newlines and decodes common entities.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else {
                break;
            };
            let tag = lower[i + 1..i + end]
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("");
            if matches!(tag, "script" | "style") && !lower[i + 1..].starts_with('/') {
                let close = format!("</{tag}");
                match lower[i..].find(&close) {
                    Some(pos) => {
                        let after = i + pos;
                        i = lower[after..].find('>').map_or(html.len(), |e| after + e + 1);
                    }
                    None => i = html.len(),
                }
                continue;
            }
            if matches!(tag, "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "table") {
                out.push('\n');
            }
            i += end + 1;
        } else if let Some(entity_rest) = rest.strip_prefix('&') {
            if let Some((c, e)) =
                entity_rest.find(';').filter(|e| *e <= 8).and_then(|e| decode_entity(&entity_rest[..e]).map(|c| (c, e)))
            {
                out.push(c);
                i += e + 2;
            } else {
                out.push('&');
                i += 1;
            }
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    collapse_blank_lines(&out)
}

/// Trims each line, collapses runs of spaces, and keeps at most one blank line in a row.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blank = false;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !blank && !out.is_empty() {
                out.push('\n');
            }
            blank = true;
        } else {
            out.push_str(&line);
            out.push('\n');
            blank = false;
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_and_control_characters_are_removed() {
        assert_eq!(neutralize("a\u{202E}b\u{2066}c\u{200F}d\u{7}e\nf\tg"), "abcde\nf\tg");
        assert_eq!(one_line("  Invoice \u{202E}\n\t 42\u{0}  "), "Invoice 42");
        assert_eq!(one_line("\u{202E}\u{2069}"), "");
    }

    #[test]
    fn truncation_respects_characters() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_bytes("héllo", 2), "h", "é is two bytes; cut before it");
        assert_eq!(truncate_bytes("héllo", 3), "hé");
        assert_eq!(truncate_bytes("abc", 10), "abc");
    }

    #[test]
    fn html_becomes_text() {
        let html = "<html><head><style>p{color:red}</style></head><body><p>Hello&nbsp;<b>world</b> &amp; friends</p>\
<script>alert('x')</script><div>Line<br>two</div><a href=\"x\">link</a> &#65;&#x42;&bogus; &lt;tag&gt;</body></html>";
        assert_eq!(html_to_text(html), "Hello world & friends\n\nLine\ntwo\nlink AB&bogus; <tag>");
    }

    #[test]
    fn html_edge_cases_never_panic() {
        for html in [
            "",
            "<",
            "<<>>",
            "<script>",
            "<style",
            "a & b",
            "&",
            "&;",
            "&#99999999999;",
            "<p",
            "é<é>é",
            "<SCRIPT>x</SCRIPT>y",
        ] {
            drop(html_to_text(html));
        }
        assert_eq!(html_to_text("<SCRIPT>x</SCRIPT>y"), "y");
    }
}

/// Unix seconds → `2026-10-05T14:30:00Z` (UTC), for the dates handed to an AI.
pub fn iso_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 {
        mp + 3
    } else {
        mp - 9
    };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs / 3_600, secs % 3_600 / 60, secs % 60)
}

/// `2026-10-05` or `2026-10-05T14:00:00+02:00` (or `Z`, or fractional seconds) → unix seconds. A bare date is midnight UTC.
pub fn parse_when(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', 't', ' ']).map_or((text, None), |(d, r)| (d, Some(r)));
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(1970..=9999).contains(&year)
    {
        return None;
    }
    // Days-from-civil.
    let shifted_year = if month <= 2 {
        year - 1
    } else {
        year
    };
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.rem_euclid(400);
    let day_of_year =
        (153 * (if month > 2 {
            month - 3
        } else {
            month + 9
        }) + 2)
            / 5
            + day
            - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let mut secs = days * 86_400;
    if let Some(rest) = rest {
        let (clock, offset) = match rest.find(['Z', 'z', '+']).or_else(|| rest.rfind('-')) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let mut clock_parts = clock.split(':');
        let hours: i64 = clock_parts.next()?.parse().ok()?;
        let minutes: i64 = clock_parts.next().unwrap_or("0").parse().ok()?;
        let seconds: i64 = clock_parts.next().unwrap_or("0").split('.').next()?.parse().ok()?;
        if hours > 23 || minutes > 59 || seconds > 60 {
            return None;
        }
        secs += hours * 3_600 + minutes * 60 + seconds;
        if let Some(sign) = offset.chars().next().filter(|c| matches!(c, '+' | '-')) {
            let mut offset_parts = offset[1..].split(':');
            let offset_hours: i64 = offset_parts.next()?.parse().ok()?;
            let offset_minutes: i64 = offset_parts.next().unwrap_or("0").parse().ok()?;
            let shift = offset_hours * 3_600 + offset_minutes * 60;
            secs -= if sign == '+' {
                shift
            } else {
                -shift
            };
        }
    }
    Some(secs)
}

#[cfg(test)]
mod time_tests {
    use super::*;

    #[test]
    fn dates_round_trip_through_text() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_utc(1_782_000_000), "2026-06-21T00:00:00Z");
        assert_eq!(iso_utc(-1), "1969-12-31T23:59:59Z");
        for t in [0, 951_782_400, 1_700_000_000, 4_102_444_799] {
            assert_eq!(parse_when(&iso_utc(t)), Some(t), "{t}");
        }
    }

    #[test]
    fn times_with_offsets_and_bare_dates_parse() {
        assert_eq!(parse_when("2023-11-14"), Some(1_699_920_000));
        assert_eq!(parse_when("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        assert_eq!(parse_when("2023-11-15T00:13:20+02:00"), Some(1_700_000_000));
        assert_eq!(parse_when("2023-11-14T17:13:20-05:00"), Some(1_700_000_000));
        assert_eq!(parse_when("2023-11-14T22:13:20.750Z"), Some(1_700_000_000));
        assert_eq!(parse_when("2023-11-14 22:13"), Some(1_699_999_980));
        for bad in ["", "tomorrow", "2023-13-01", "2023-11-32", "1960-01-01", "2023-11-14T25:00:00Z", "2023-11"] {
            assert_eq!(parse_when(bad), None, "{bad:?}");
        }
    }
}
