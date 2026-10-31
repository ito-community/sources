//! The site's ISO 8601 timestamps as seconds since the Unix epoch, which is what Ito stores for
//! chapter dates.

/// Parses `YYYY-MM-DDTHH:MM:SS`, optionally with fractional seconds, followed by `Z` or a
/// `±HH:MM` offset (no suffix means UTC). Anything else, including impossible dates, is `None`.
pub fn parse_iso(text: &str) -> Option<f64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', ' '])?;
    let mut date = date.split('-');
    let year: i64 = number(date.next()?, 4)?;
    let month: i64 = number(date.next()?, 2)?;
    let day: i64 = number(date.next()?, 2)?;
    if date.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if !(1..=days_in_month(year, month)).contains(&day) {
        return None;
    }
    let (clock, offset) = split_offset(rest)?;
    let mut clock = clock.split(':');
    let hour: i64 = number(clock.next()?, 2)?;
    let minute: i64 = number(clock.next()?, 2)?;
    let seconds = clock.next()?;
    if clock.next().is_some() || hour > 23 || minute > 59 {
        return None;
    }
    let (whole, fraction) = match seconds.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (seconds, None),
    };
    let second: i64 = number(whole, 2)?;
    if second > 60 {
        return None;
    }
    let fraction = match fraction {
        Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            format!("0.{digits}").parse::<f64>().ok()?
        }
        Some(_) => return None,
        None => 0.0,
    };
    let days = days_from_civil(year, month, day);
    let local = days * 86_400 + hour * 3_600 + minute * 60 + second;
    Some((local - offset) as f64 + fraction)
}

/// Separates `Z` or `±HH:MM` from the clock time; the offset in seconds east of UTC.
fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
        return Some((clock, 0));
    }
    if let Some(at) = rest.rfind(['+', '-']) {
        let (clock, offset) = rest.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (hours, minutes) = offset[1..].split_once(':')?;
        let hours: i64 = number(hours, 2)?;
        let minutes: i64 = number(minutes, 2)?;
        if hours > 23 || minutes > 59 {
            return None;
        }
        return Some((clock, sign * (hours * 3_600 + minutes * 60)));
    }
    Some((rest, 0))
}

/// A fixed-width run of ASCII digits.
fn number(text: &str, width: usize) -> Option<i64> {
    (text.len() == width && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_timestamps_convert_to_epoch_seconds() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(parse_iso("2000-03-01T00:00:00Z"), Some(951_868_800.0));
        assert_eq!(
            parse_iso("2025-11-02T19:32:15.931Z"),
            Some(1_762_111_935.931)
        );
        // A leap day exists only in leap years.
        assert_eq!(parse_iso("2024-02-29T12:00:00Z"), Some(1_709_208_000.0));
        assert_eq!(parse_iso("2023-02-29T12:00:00Z"), None);
        assert_eq!(parse_iso("1900-02-29T00:00:00Z"), None);
        assert_eq!(parse_iso("1969-12-31T23:59:59Z"), Some(-1.0));
    }

    #[test]
    fn offsets_shift_to_utc() {
        assert_eq!(
            parse_iso("2025-01-01T01:00:00+01:00"),
            parse_iso("2025-01-01T00:00:00Z")
        );
        assert_eq!(
            parse_iso("2025-01-01T00:00:00-05:30"),
            parse_iso("2025-01-01T05:30:00Z")
        );
        assert_eq!(
            parse_iso("2025-01-01T00:00:00"),
            parse_iso("2025-01-01T00:00:00Z")
        );
    }

    #[test]
    fn malformed_timestamps_are_not_dates() {
        for text in [
            "",
            "2025-13-01T00:00:00Z",
            "2025-00-10T00:00:00Z",
            "2025-01-32T00:00:00Z",
            "2025-01-01T24:00:00Z",
            "2025-01-01T00:60:00Z",
            "2025-01-01T00:00:61Z",
            "2025-01-01",
            "2025-1-1T00:00:00Z",
            "2025-01-01T00:00Z",
            "2025-01-01T00:00:00.Z",
            "2025-01-01T00:00:00.x1Z",
            "yesterday",
            "2025-01-01T00:00:00+0100",
        ] {
            assert_eq!(parse_iso(text), None, "{text:?}");
        }
    }
}
