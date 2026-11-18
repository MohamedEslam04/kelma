//! Minimal ISO-8601 timestamp parsing (Teams sends `2026-11-18T11:24:38.0520781Z`).

/// Parses an ISO-8601 UTC timestamp into Unix milliseconds. Returns `None` on malformed input.
///
/// Accepts an optional fractional part of any length and a trailing `Z` or `+00:00`.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || (b[10] != b'T' && b[10] != b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut ms = 0i64;
    let rest = &s[19..];
    let mut tz = rest;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
        tz = &frac[digits.len()..];
        let mut first3: String = digits.chars().take(3).collect();
        while first3.len() < 3 {
            first3.push('0');
        }
        ms = first3.parse().ok()?;
    }
    let offset_min = match tz {
        "" | "Z" | "z" => 0,
        t if (t.starts_with('+') || t.starts_with('-')) && t.len() == 6 && &t[3..4] == ":" => {
            let sign = if t.starts_with('-') { -1 } else { 1 };
            sign * (t[1..3].parse::<i64>().ok()? * 60 + t[4..6].parse::<i64>().ok()?)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 86_400 + h * 3600 + mi * 60 + se) - offset_min * 60) * 1000 + ms)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Current Unix time in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_teams_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2026-11-18T11:24:38.0520781Z"),
            Some(1_795_001_078_052)
        );
        assert_eq!(
            parse_iso_ms("2026-11-18T11:24:38.5Z"),
            Some(1_795_001_078_500)
        );
        assert_eq!(
            parse_iso_ms("2026-11-18T13:24:38+02:00"),
            Some(1_795_001_078_000)
        );
        assert_eq!(parse_iso_ms("2000-02-29T00:00:00Z"), Some(951_782_400_000));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_iso_ms(""), None);
        assert_eq!(parse_iso_ms("not a date at all!!"), None);
        assert_eq!(parse_iso_ms("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso_ms("2026-11-18T11:24:38Q"), None);
    }
}
