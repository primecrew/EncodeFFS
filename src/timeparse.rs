//! Time entry (1.1): every field that takes a time in seconds also accepts mm:ss and hh:mm:ss.
//! The value is stored and passed to FFmpeg as plain seconds, exactly as before.
//!
//! Accepted: `84.5` (seconds), `12:30` / `1:05.5` (mm:ss), `1:02:03` / `1:02:03.25` (hh:mm:ss).
//! Rejected with a plain message: letters, signs, empty parts, minutes or seconds of 60 or more in
//! the colon forms, more than two colons.

const USE: &str = "Use seconds, mm:ss or hh:mm:ss (like 84.5, 1:24.5 or 1:02:03).";

/// Plain seconds from what the user typed. Empty = 0.
pub fn parse_time(text: &str) -> Result<f64, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(0.0);
    }
    let fail = |why: &str| format!("“{t}” isn't a valid time. {why}{USE}");
    if t.starts_with('-') {
        return Err(fail("A time can't be negative. "));
    }
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() > 3 {
        return Err(fail("It has too many colons. "));
    }
    let bad = || fail("");
    let secs_part = parts[parts.len() - 1];
    let secs = number(secs_part).ok_or_else(bad)?;
    if parts.len() == 1 {
        return Ok(secs);
    }
    if secs >= 60.0 {
        return Err(fail("Seconds must be below 60. "));
    }
    let whole = |p: &str| -> Option<u64> { (!p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).then(|| p.parse().ok()).flatten() };
    let minutes = whole(parts[parts.len() - 2]).ok_or_else(bad)?;
    if minutes >= 60 {
        return Err(if parts.len() == 2 {
            fail("Minutes must be below 60. For an hour or more use hh:mm:ss (like 1:30:00). ")
        } else {
            fail("Minutes must be below 60. ")
        });
    }
    let hours = if parts.len() == 3 { whole(parts[0]).ok_or_else(bad)? } else { 0 };
    Ok(hours as f64 * 3600.0 + minutes as f64 * 60.0 + secs)
}

/// Digits with at most one decimal point ("5", "5.", ".5", "84.25"); no signs or exponents.
fn number(s: &str) -> Option<f64> {
    if s.is_empty() || s == "." || !s.chars().all(|c| c.is_ascii_digit() || c == '.') || s.matches('.').count() > 1 {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Seconds as typed in a field: "0", "84.5", "3723" (up to 3 decimals, no trailing zeros).
pub fn plain_seconds(v: f64) -> String {
    let s = format!("{:.3}", v.max(0.0));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// What a time box shows for a stored value: plain seconds below a minute ("12", "84.5" shows as
/// "1:24.5"), the clock form from a minute up ("2:15", "1:02:03"). Parses back to the same value.
pub fn field_text(v: f64) -> String {
    if v >= 60.0 { clock(v) } else { plain_seconds(v) }
}

/// Clock form: "0:12", "1:24.5", "1:02:03", "1:02:03.25" (up to 3 decimals).
pub fn clock(v: f64) -> String {
    let ms_total = (v.max(0.0) * 1000.0).round() as u64;
    let ms = ms_total % 1000;
    let s = (ms_total / 1000) % 60;
    let m = (ms_total / 60_000) % 60;
    let h = ms_total / 3_600_000;
    let frac = if ms == 0 { String::new() } else { format!(".{ms:03}").trim_end_matches('0').to_string() };
    if h > 0 { format!("{h}:{m:02}:{s:02}{frac}") } else { format!("{m}:{s:02}{frac}") }
}

/// "1:02:03 (3,723 s)" — the parsed value in plain words, shown next to the field.
pub fn describe(v: f64) -> String {
    let secs = plain_seconds(v);
    let (int, frac) = secs.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((secs.clone(), String::new()));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{} ({grouped}{frac} s)", clock(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_seconds_forms() {
        for (t, v) in [("0", 0.0), ("84.5", 84.5), (" 12 ", 12.0), ("5.", 5.0), (".5", 0.5), ("3723", 3723.0), ("", 0.0), ("   ", 0.0), ("007", 7.0)] {
            assert_eq!(parse_time(t), Ok(v), "{t:?}");
        }
    }

    #[test]
    fn colon_forms() {
        for (t, v) in [
            ("12:30", 750.0),
            ("1:05.5", 65.5),
            ("0:00", 0.0),
            ("00:59.999", 59.999),
            ("59:59", 3599.0),
            ("1:02:03", 3723.0),
            ("1:02:03.25", 3723.25),
            ("0:00:01", 1.0),
            ("10:00:00", 36000.0),
            ("123:00:00", 442800.0),
            (" 1:02:03 ", 3723.0),
        ] {
            let got = parse_time(t).unwrap();
            assert!((got - v).abs() < 1e-9, "{t}: {got} vs {v}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        for t in ["abc", "1m30s", "1:2:3:4", "1::2", ":30", "1:", "1:60", "1:59:60", "1:60:00", "60:00", "90:00", "-5", "-1:00", "+5", "1e3", "1.2.3", "1:2.5:3", ".", "1,5", "1: 30", "inf", "NaN", "1:-5"] {
            assert!(parse_time(t).is_err(), "{t:?} should be rejected");
        }
        assert!(parse_time("-5").unwrap_err().contains("negative"));
        assert!(parse_time("1:2:3:4").unwrap_err().contains("too many colons"));
        assert!(parse_time("1:75").unwrap_err().contains("Seconds must be below 60"));
        assert!(parse_time("90:00").unwrap_err().contains("hh:mm:ss"));
        assert!(parse_time("1:75:00").unwrap_err().contains("Minutes must be below 60"));
        assert!(parse_time("abc").unwrap_err().contains("seconds, mm:ss or hh:mm:ss"));
        assert_eq!(parse_time("1:75").unwrap_err(), "“1:75” isn't a valid time. Seconds must be below 60. Use seconds, mm:ss or hh:mm:ss (like 84.5, 1:24.5 or 1:02:03).");
        assert_eq!(parse_time("12a").unwrap_err(), "“12a” isn't a valid time. Use seconds, mm:ss or hh:mm:ss (like 84.5, 1:24.5 or 1:02:03).");
    }

    #[test]
    fn display_and_round_trip() {
        assert_eq!(describe(3723.0), "1:02:03 (3,723 s)");
        assert_eq!(describe(84.5), "1:24.5 (84.5 s)");
        assert_eq!(describe(0.0), "0:00 (0 s)");
        assert_eq!(describe(12.0), "0:12 (12 s)");
        assert_eq!(describe(1234567.25), "342:56:07.25 (1,234,567.25 s)");
        assert_eq!(clock(59.9995), "1:00");
        assert_eq!(plain_seconds(84.5), "84.5");
        assert_eq!(plain_seconds(0.1), "0.1");
        assert_eq!(plain_seconds(10.0), "10");
        assert_eq!(field_text(12.0), "12");
        assert_eq!(field_text(0.5), "0.5");
        assert_eq!(field_text(135.0), "2:15");
        assert_eq!(field_text(3723.0), "1:02:03");
        for v in [0.0, 0.1, 59.999, 60.0, 84.5, 135.0, 3599.5, 3723.25, 86400.0] {
            assert!((parse_time(&field_text(v)).unwrap() - v).abs() < 1e-9, "{v}");
        }
        // what a field shows parses back to the same value
        for v in [0.0, 0.1, 2.5, 84.5, 3723.0, 3723.25, 86400.0] {
            assert_eq!(parse_time(&plain_seconds(v)).unwrap(), v);
            assert!((parse_time(&clock(v)).unwrap() - v).abs() < 1e-9, "{v}");
        }
    }
}
