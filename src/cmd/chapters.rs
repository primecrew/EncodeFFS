//! Chapter generation (FFMETADATA). Times are relative to the *trimmed output*,
//! which is what ffmpeg sees when the chapter file is a separate input and
//! `-ss` is applied only to the media input.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChapterMode {
    #[default]
    Off,
    /// Automatic every n minutes.
    Auto,
    /// Manual chapters only.
    Manual,
    /// Automatic + manual.
    Both,
}

impl ChapterMode {
    pub const ALL: [ChapterMode; 4] = [ChapterMode::Off, ChapterMode::Auto, ChapterMode::Manual, ChapterMode::Both];
    pub fn label(self) -> &'static str {
        match self {
            ChapterMode::Off => "No chapters",
            ChapterMode::Auto => "Automatic every n minutes",
            ChapterMode::Manual => "Manual chapters only",
            ChapterMode::Both => "Automatic + manual",
        }
    }
    fn auto(self) -> bool {
        matches!(self, ChapterMode::Auto | ChapterMode::Both)
    }
    fn manual(self) -> bool {
        matches!(self, ChapterMode::Manual | ChapterMode::Both)
    }
}

/// A manual chapter as typed by the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ManualChapter {
    /// "hh:mm:ss" (also accepts h:mm:ss and fractional seconds).
    pub time: String,
    pub title: String,
}

/// A resolved chapter in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
}

/// Parse "hh:mm:ss" (the script's format) into milliseconds. Also accepts
/// "h:mm:ss", "mm:ss" and a fractional seconds part ("00:01:02.5").
pub fn parse_hms(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!("Invalid format: {s}. Use hh:mm:ss (e.g., 00:15:30)"));
    }
    let (h, m, sec) = if parts.len() == 3 { (parts[0], parts[1], parts[2]) } else { ("0", parts[0], parts[1]) };
    let h: i64 = h.parse().map_err(|_| format!("Invalid hours in {s}"))?;
    let m: i64 = m.parse().map_err(|_| format!("Invalid minutes in {s}"))?;
    let sec: f64 = sec.parse().map_err(|_| format!("Invalid seconds in {s}"))?;
    if h < 0 || !(0..60).contains(&m) || !(0.0..60.0).contains(&sec) {
        return Err(format!("Invalid time: {s}. Minutes and seconds must be less than 60"));
    }
    Ok(h * 3_600_000 + m * 60_000 + (sec * 1000.0).round() as i64)
}

/// Build the chapter list. Mirrors New-ChapterFile in the script:
/// auto chapters every `interval_s` from 0 while < output duration, manual
/// chapters must be inside the output, list sorted and de-duplicated by start
/// time (a manual chapter wins over an automatic one at the same time), END is
/// the next START or the output duration.
pub fn build_chapters(
    output_duration_s: f64,
    mode: ChapterMode,
    interval_s: f64,
    manual: &[ManualChapter],
) -> Result<Vec<Chapter>, String> {
    let end_total = (output_duration_s * 1000.0).round() as i64;
    let mut list: Vec<(i64, String, bool)> = Vec::new(); // (ms, title, is_manual)
    if mode.auto() && interval_s > 0.0 {
        let mut n = 1;
        let mut t = 0.0f64;
        while t < output_duration_s {
            list.push(((t * 1000.0).round() as i64, format!("Chapter {n}"), false));
            n += 1;
            t += interval_s;
        }
    }
    if mode.manual() {
        for (i, mc) in manual.iter().enumerate() {
            if mc.time.trim().is_empty() {
                continue;
            }
            let ms = parse_hms(&mc.time)?;
            let title = if mc.title.trim().is_empty() { format!("Chapter {}", i + 1) } else { mc.title.trim().to_string() };
            if ms >= end_total {
                return Err(format!(
                    "Manual chapter '{}' ({}) is outside output duration ({} seconds)",
                    title, mc.time, crate::util::fmt_num(output_duration_s)
                ));
            }
            list.push((ms, title, true));
        }
    }
    // Sort by time, manual first on ties, then keep the first per time.
    list.sort_by(|a, b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)));
    list.dedup_by_key(|c| c.0);

    let mut out = Vec::with_capacity(list.len());
    for i in 0..list.len() {
        let start = list[i].0;
        let end = if i + 1 < list.len() { list[i + 1].0 } else { end_total };
        if start >= end {
            return Err(format!("Invalid chapter: START={start}, END={end}, Title={}", list[i].1));
        }
        out.push(Chapter { start_ms: start, end_ms: end, title: list[i].1.clone() });
    }
    Ok(out)
}

/// Escape a value for FFMETADATA ('=', ';', '#', '\' and newlines need a backslash).
fn escape_meta(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// FFMETADATA1 text for the chapters (LF line endings, UTF-8 without BOM).
pub fn ffmetadata(chapters: &[Chapter]) -> String {
    let mut s = String::from(";FFMETADATA1\n");
    for c in chapters {
        s.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n",
            c.start_ms,
            c.end_ms,
            escape_meta(&c.title)
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hms_parsing() {
        assert_eq!(parse_hms("00:15:30").unwrap(), 930_000);
        assert_eq!(parse_hms("1:00:00").unwrap(), 3_600_000);
        assert_eq!(parse_hms("02:03").unwrap(), 123_000);
        assert_eq!(parse_hms("00:00:01.5").unwrap(), 1_500);
        assert!(parse_hms("00:60:00").is_err());
        assert!(parse_hms("abc").is_err());
    }

    #[test]
    fn auto_chapters() {
        let c = build_chapters(700.0, ChapterMode::Auto, 300.0, &[]).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].start_ms, c[0].end_ms), (0, 300_000));
        assert_eq!((c[2].start_ms, c[2].end_ms, c[2].title.as_str()), (600_000, 700_000, "Chapter 3"));
    }

    #[test]
    fn manual_and_merge() {
        let manual = vec![
            ManualChapter { time: "00:05:00".into(), title: "Boss".into() },
            ManualChapter { time: "00:02:00".into(), title: "".into() },
        ];
        let c = build_chapters(700.0, ChapterMode::Both, 300.0, &manual).unwrap();
        let titles: Vec<_> = c.iter().map(|c| (c.start_ms, c.title.clone())).collect();
        assert_eq!(
            titles,
            vec![(0, "Chapter 1".into()), (120_000, "Chapter 2".into()), (300_000, "Boss".into()), (600_000, "Chapter 3".into())]
        );
        // manual-only ignores interval
        let c = build_chapters(700.0, ChapterMode::Manual, 300.0, &manual).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].end_ms, 700_000);
    }

    #[test]
    fn manual_outside_duration_errors() {
        let manual = vec![ManualChapter { time: "00:20:00".into(), title: "x".into() }];
        assert!(build_chapters(700.0, ChapterMode::Manual, 0.0, &manual).is_err());
    }

    #[test]
    fn metadata_text() {
        let c = vec![Chapter { start_ms: 0, end_ms: 1000, title: "A=B; #1".into() }];
        assert_eq!(
            ffmetadata(&c),
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=A\\=B\\; \\#1\n"
        );
    }

    #[test]
    fn off_gives_nothing() {
        assert!(build_chapters(100.0, ChapterMode::Off, 300.0, &[]).unwrap().is_empty());
    }
}
