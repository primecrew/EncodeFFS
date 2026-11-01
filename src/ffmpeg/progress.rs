//! Parsing of `-progress pipe:1` key=value output and stderr "Duration:" lines.

#[derive(Debug, Clone, PartialEq)]
pub enum ProgressEvent {
    /// Output time position in seconds.
    OutTime(f64),
    Speed(String),
    End,
}

/// Parse one line of `-progress` output.
pub fn parse_line(line: &str) -> Option<ProgressEvent> {
    let (k, v) = line.trim().split_once('=')?;
    let v = v.trim();
    match k {
        // out_time_ms is (historically) also in microseconds.
        "out_time_us" | "out_time_ms" => v.parse::<i64>().ok().filter(|x| *x >= 0).map(|us| ProgressEvent::OutTime(us as f64 / 1e6)),
        "out_time" => parse_clock(v).map(ProgressEvent::OutTime),
        "speed" if v != "N/A" => Some(ProgressEvent::Speed(v.to_string())),
        "progress" if v == "end" => Some(ProgressEvent::End),
        _ => None,
    }
}

/// "HH:MM:SS.micro" -> seconds
pub fn parse_clock(s: &str) -> Option<f64> {
    let mut it = s.split(':');
    let h: f64 = it.next()?.trim().parse().ok()?;
    let m: f64 = it.next()?.trim().parse().ok()?;
    let sec: f64 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || h < 0.0 {
        return None;
    }
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// "  Duration: 00:01:02.50, start: 0.000000, bitrate: ..." -> 62.5
pub fn parse_duration_line(line: &str) -> Option<f64> {
    let rest = line.trim_start().strip_prefix("Duration:")?;
    let clock = rest.trim().split(',').next()?;
    parse_clock(clock)
}

/// Fraction 0..=1 given position and total.
pub fn fraction(pos: f64, total: Option<f64>) -> Option<f32> {
    let t = total.filter(|t| *t > 0.0)?;
    Some((pos / t).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lines() {
        assert_eq!(parse_line("out_time_us=1500000"), Some(ProgressEvent::OutTime(1.5)));
        assert_eq!(parse_line("out_time_ms=2000000"), Some(ProgressEvent::OutTime(2.0)));
        assert_eq!(parse_line("out_time=00:01:00.500000"), Some(ProgressEvent::OutTime(60.5)));
        assert_eq!(parse_line("out_time_us=N/A"), None);
        assert_eq!(parse_line("speed=2.5x"), Some(ProgressEvent::Speed("2.5x".into())));
        assert_eq!(parse_line("speed=N/A"), None);
        assert_eq!(parse_line("progress=end"), Some(ProgressEvent::End));
        assert_eq!(parse_line("progress=continue"), None);
        assert_eq!(parse_line("frame=10"), None);
    }
    #[test]
    fn duration_line() {
        assert_eq!(parse_duration_line("  Duration: 00:01:02.50, start: 0.000000, bitrate: 128 kb/s"), Some(62.5));
        assert_eq!(parse_duration_line("  Duration: N/A, start"), None);
        assert_eq!(fraction(5.0, Some(10.0)), Some(0.5));
        assert_eq!(fraction(15.0, Some(10.0)), Some(1.0));
        assert_eq!(fraction(5.0, None), None);
    }
}
