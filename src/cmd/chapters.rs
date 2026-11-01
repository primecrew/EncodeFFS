//! Chapter generation (FFMETADATA). Times are relative to the *trimmed output*,
//! which is what ffmpeg sees when the chapter file is a separate input and
//! `-ss` is applied only to the media input.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Temp chapter files are named `ffenc_chapters_<id>.ffmeta` in the temp folder.
pub const TEMP_PREFIX: &str = "ffenc_chapters_";
pub const TEMP_EXT: &str = "ffmeta";

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
    pub fn auto(self) -> bool {
        matches!(self, ChapterMode::Auto | ChapterMode::Both)
    }
    pub fn manual(self) -> bool {
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
        let mut t = 0.0f64;
        // no automatic chapter in the last second (avoids 4 ms tail chapters from rounding)
        while t < output_duration_s && (t == 0.0 || output_duration_s - t >= 1.0) {
            list.push(((t * 1000.0).round() as i64, String::new(), false));
            t += interval_s;
        }
    }
    if mode.manual() {
        for (i, mc) in manual.iter().enumerate() {
            if mc.time.trim().is_empty() {
                continue;
            }
            let ms = parse_hms(&mc.time)?;
            let title = mc.title.trim().to_string();
            let shown = if title.is_empty() { format!("#{} ({})", i + 1, mc.time.trim()) } else { title.clone() };
            if list.iter().any(|c| c.2 && c.0 == ms) {
                return Err(format!("Two manual chapters start at the same time ({})", mc.time.trim()));
            }
            if ms >= end_total {
                return Err(format!(
                    "Manual chapter '{}' ({}) is outside output duration ({} seconds)",
                    shown, mc.time, crate::util::fmt_num(output_duration_s)
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
        // untitled chapters are numbered by their final position
        let title = if list[i].1.is_empty() { format!("Chapter {}", i + 1) } else { list[i].1.clone() };
        out.push(Chapter { start_ms: start, end_ms: end, title });
    }
    Ok(out)
}

/// Escape a value for FFMETADATA ('=', ';', '#', '\' and newlines need a backslash).
/// CR/CRLF inside a title become a single (escaped) LF.
pub fn escape_meta(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
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
    ffmetadata_with(chapters, false)
}

/// FFMETADATA1 text; `crlf` switches every line ending to CRLF (never mixed).
/// Format (FFmpeg docs, "Metadata"): first line `;FFMETADATA1`, then per chapter
/// `[CHAPTER]`, `TIMEBASE=1/1000`, `START=<int>`, `END=<int>`, `title=<escaped>`,
/// no spaces around `=`.
pub fn ffmetadata_with(chapters: &[Chapter], crlf: bool) -> String {
    let nl = if crlf { "\r\n" } else { "\n" };
    let mut s = format!(";FFMETADATA1{nl}");
    for c in chapters {
        for line in ["[CHAPTER]".to_string(), "TIMEBASE=1/1000".into(), format!("START={}", c.start_ms), format!("END={}", c.end_ms), format!("title={}", escape_meta(&c.title))] {
            s.push_str(&line);
            s.push_str(nl);
        }
    }
    s
}

/// Undo `escape_meta`.
pub fn unescape_meta(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                o.push(n);
            }
        } else {
            o.push(c);
        }
    }
    o
}

/// Parse the chapters of an FFMETADATA1 text (TIMEBASE honoured, times returned in ms).
/// Used for round-trip checks and to read back chapter files.
pub fn parse_ffmetadata(text: &str) -> Result<Vec<Chapter>, String> {
    let text = text.replace("\r\n", "\n");
    let mut lines = text.split('\n');
    if lines.next().map(|l| l.trim_end()) != Some(";FFMETADATA1") {
        return Err("Missing ;FFMETADATA1 header".into());
    }
    // join escaped newlines (a line ending in an odd number of backslashes continues)
    let mut logical: Vec<String> = vec![];
    let mut cont = false;
    for l in lines {
        if cont {
            let last = logical.last_mut().unwrap();
            last.push('\n');
            last.push_str(l);
        } else {
            logical.push(l.to_string());
        }
        let trailing = logical.last().unwrap().chars().rev().take_while(|c| *c == '\\').count();
        cont = trailing % 2 == 1;
    }
    let mut out: Vec<Chapter> = vec![];
    let mut cur: Option<(i64, i64, i64, i64, String)> = None; // num, den, start, end, title
    let finish = |cur: &mut Option<(i64, i64, i64, i64, String)>, out: &mut Vec<Chapter>| -> Result<(), String> {
        if let Some((n, d, s, e, t)) = cur.take() {
            if d == 0 || n <= 0 {
                return Err("Invalid TIMEBASE".into());
            }
            let ms = |v: i64| ((v as i128 * n as i128 * 1000) as f64 / d as f64).round() as i64;
            out.push(Chapter { start_ms: ms(s), end_ms: ms(e), title: t });
        }
        Ok(())
    };
    for l in logical {
        if l.is_empty() || l.starts_with(';') || l.starts_with('#') {
            continue;
        }
        if l.starts_with('[') {
            finish(&mut cur, &mut out)?;
            if l.trim_end() == "[CHAPTER]" {
                cur = Some((1, 1000, 0, 0, String::new()));
            }
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        // split on the first unescaped '='
        let mut key = String::new();
        let mut rest = None;
        let mut esc = false;
        for (i, ch) in l.char_indices() {
            if esc {
                esc = false;
                key.push(ch);
            } else if ch == '\\' {
                esc = true;
            } else if ch == '=' {
                rest = Some(&l[i + 1..]);
                break;
            } else {
                key.push(ch);
            }
        }
        let Some(val) = rest else { return Err(format!("Line without '=': {l}")) };
        let num = |v: &str| v.trim().parse::<i64>().map_err(|_| format!("Not an integer: {v}"));
        match key.as_str() {
            "TIMEBASE" => {
                let (n, d) = val.split_once('/').ok_or("TIMEBASE must be n/d")?;
                c.0 = num(n)?;
                c.1 = num(d)?;
            }
            "START" => c.2 = num(val)?,
            "END" => c.3 = num(val)?,
            "title" => c.4 = unescape_meta(val),
            _ => {}
        }
    }
    finish(&mut cur, &mut out)?;
    Ok(out)
}

/// Check a chapter list the way the creator promises: starts strictly
/// increasing, no zero-length chapters, inside `[0, duration]`, last END = duration.
pub fn validate(chapters: &[Chapter], output_duration_s: f64) -> Result<(), String> {
    let end_total = (output_duration_s * 1000.0).round() as i64;
    for (i, c) in chapters.iter().enumerate() {
        if c.start_ms < 0 || c.end_ms <= c.start_ms {
            return Err(format!("Chapter {} has zero or negative length", i + 1));
        }
        if i > 0 && c.start_ms <= chapters[i - 1].start_ms {
            return Err(format!("Chapter {} does not start after chapter {}", i + 1, i));
        }
        if i + 1 < chapters.len() && c.end_ms != chapters[i + 1].start_ms {
            return Err(format!("Chapter {} does not end where chapter {} starts", i + 1, i + 2));
        }
        if c.end_ms > end_total {
            return Err(format!("Chapter {} ends after the output duration", i + 1));
        }
    }
    if let Some(l) = chapters.last() {
        if l.end_ms != end_total {
            return Err("The last chapter must end at the output duration".into());
        }
    }
    Ok(())
}

/// Build + validate + render the temp chapter file for one job.
/// `None` when the mode produces no chapters.
pub fn chapter_input(output_duration_s: f64, mode: ChapterMode, interval_s: f64, manual: &[ManualChapter]) -> Result<Option<(PathBuf, String)>, String> {
    if mode == ChapterMode::Off {
        return Ok(None);
    }
    if output_duration_s <= 0.0 {
        return Err("Trim duration exceeds the input duration".into());
    }
    let list = build_chapters(output_duration_s, mode, interval_s, manual)?;
    if list.is_empty() {
        return Ok(None);
    }
    validate(&list, output_duration_s)?;
    Ok(Some((temp_chapter_path(), ffmetadata(&list))))
}

/// Unique `%TEMP%\ffenc_chapters_<id>.ffmeta`.
pub fn temp_chapter_path() -> PathBuf {
    std::env::temp_dir().join(format!("{TEMP_PREFIX}{}.{TEMP_EXT}", crate::util::unique_id()))
}

/// Delete leftover chapter files (older than `max_age`) in `dir` – e.g. after a crash.
pub fn cleanup_stale(dir: &Path, max_age: std::time::Duration) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with(TEMP_PREFIX) || !(name.ends_with(&format!(".{TEMP_EXT}")) || name.ends_with(".txt")) {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).map(|a| a >= max_age).unwrap_or(false);
        if old && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// Flexible timestamp for the "set from timestamp" helper: `750.5`, `12:30`,
/// `1:02:03.250`, `1h2m3.5s`, `90s`, `2m`. Returns milliseconds.
pub fn parse_timestamp(s: &str) -> Result<i64, String> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Err("Empty timestamp".into());
    }
    if s.contains(':') {
        return parse_hms(&s);
    }
    if let Ok(v) = s.parse::<f64>() {
        return if v >= 0.0 { Ok((v * 1000.0).round() as i64) } else { Err("Negative timestamp".into()) };
    }
    let mut total = 0.0;
    let mut num = String::new();
    let mut any = false;
    for c in s.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'h' | 'm' | 's' => {
                let v: f64 = num.parse().map_err(|_| format!("Invalid timestamp: {s}"))?;
                total += v * match c { 'h' => 3600.0, 'm' => 60.0, _ => 1.0 };
                num.clear();
                any = true;
            }
            ' ' => {}
            _ => return Err(format!("Invalid timestamp: {s}")),
        }
    }
    if !num.is_empty() || !any {
        return Err(format!("Invalid timestamp: {s}"));
    }
    Ok((total * 1000.0).round() as i64)
}

/// `hh:mm:ss` or `hh:mm:ss.mmm` when there are milliseconds.
pub fn fmt_ms(ms: i64) -> String {
    let ms = ms.max(0);
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if f == 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}.{f:03}")
    }
}

/// Import "00:12:30 Title" lines (also `00:12:30 - Title`, tabs, `0:00 Intro`).
/// Blank lines and lines starting with `#` are ignored.
pub fn parse_chapter_list(text: &str) -> Result<Vec<ManualChapter>, String> {
    let mut out = vec![];
    for (n, line) in text.lines().enumerate() {
        let l = line.trim().trim_start_matches('\u{feff}');
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (time, title) = match l.find(|c: char| c.is_whitespace()) {
            Some(i) => (&l[..i], l[i..].trim()),
            None => (l, ""),
        };
        let title = title.trim_start_matches(['-', '–', '|']).trim();
        let ms = parse_hms(time).map_err(|e| format!("Line {}: {e}", n + 1))?;
        out.push(ManualChapter { time: fmt_ms(ms), title: title.to_string() });
    }
    Ok(out)
}

/// Existing chapters of the source as manual entries relative to the trimmed output.
pub fn from_source(chapters: &[(f64, String)], trim_start: f64, output_duration_s: f64) -> Vec<ManualChapter> {
    chapters
        .iter()
        .filter_map(|(start, title)| {
            let t = start - trim_start;
            (t >= 0.0 && t < output_duration_s).then(|| ManualChapter { time: fmt_ms((t * 1000.0).round() as i64), title: title.clone() })
        })
        .collect()
}

/// Sort manual entries by time (unparsable times last).
pub fn sort_manual(manual: &mut [ManualChapter]) {
    manual.sort_by_key(|m| parse_hms(&m.time).unwrap_or(i64::MAX));
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
            vec![(0, "Chapter 1".into()), (120_000, "Chapter 2".into()), (300_000, "Boss".into()), (600_000, "Chapter 4".into())]
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
    fn no_tiny_tail_chapter() {
        let c = build_chapters(600.004, ChapterMode::Auto, 300.0, &[]).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].end_ms, 600_004);
        assert_eq!(build_chapters(0.5, ChapterMode::Auto, 300.0, &[]).unwrap().len(), 1);
    }

    #[test]
    fn off_gives_nothing() {
        assert!(build_chapters(100.0, ChapterMode::Off, 300.0, &[]).unwrap().is_empty());
    }

    #[test]
    fn escaping_every_special_char() {
        let c = vec![
            Chapter { start_ms: 0, end_ms: 1500, title: "a=b;c#d\\e\nf ü 視頻".into() },
            Chapter { start_ms: 1500, end_ms: 3000, title: "crlf\r\nx\ry".into() },
        ];
        let lf = ffmetadata(&c);
        assert_eq!(
            lf,
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1500\ntitle=a\\=b\\;c\\#d\\\\e\\\nf ü 視頻\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=1500\nEND=3000\ntitle=crlf\\\nx\\\ny\n"
        );
        assert!(!lf.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]), "no BOM");
        assert!(!lf.contains('\r'));
        assert!(!lf.contains(" =") && !lf.contains("= "));
        // CRLF variant: every line ends with CRLF, none with a bare LF
        let crlf = ffmetadata_with(&c, true);
        assert_eq!(crlf.matches("\r\n").count(), crlf.matches('\n').count() - 3, "only the three escaped title newlines are bare LFs");
        // round trip
        let back = parse_ffmetadata(&lf).unwrap();
        assert_eq!(back[0].title, "a=b;c#d\\e\nf ü 視頻");
        assert_eq!(back[1].title, "crlf\nx\ny");
        assert_eq!((back[1].start_ms, back[1].end_ms), (1500, 3000));
        assert_eq!(parse_ffmetadata(&crlf).unwrap(), back);
        assert!(parse_ffmetadata("[CHAPTER]\n").is_err());
        assert_eq!(parse_ffmetadata(";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/90000\nSTART=90000\nEND=180000\ntitle=x\n").unwrap()[0].start_ms, 1000);
    }

    #[test]
    fn integers_and_rounding() {
        // 0.0004 s rounds down, 2.0005 rounds up; output duration 3.3337 -> 3334 ms
        let m = vec![ManualChapter { time: "00:00:02.0005".into(), title: "x".into() }];
        let c = build_chapters(3.3337, ChapterMode::Both, 1.0, &m).unwrap();
        let starts: Vec<i64> = c.iter().map(|c| c.start_ms).collect();
        assert_eq!(starts, vec![0, 1000, 2000, 2001], "auto 3000 dropped: < 1 s before the end");
        assert_eq!(c.last().unwrap().end_ms, 3334);
        assert!(validate(&c, 3.3337).is_ok());
        let text = ffmetadata(&c);
        for l in text.lines().filter(|l| l.starts_with("START=") || l.starts_with("END=")) {
            assert!(l.split('=').nth(1).unwrap().chars().all(|c| c.is_ascii_digit()), "{l}");
        }
    }

    #[test]
    fn validation_rules() {
        let ok = vec![Chapter { start_ms: 0, end_ms: 1000, title: "a".into() }, Chapter { start_ms: 1000, end_ms: 2000, title: "b".into() }];
        assert!(validate(&ok, 2.0).is_ok());
        assert!(validate(&ok, 3.0).unwrap_err().contains("last chapter"));
        let mut z = ok.clone();
        z[1].start_ms = 1000;
        z[1].end_ms = 1000;
        assert!(validate(&z, 1.0).is_err());
        let gap = vec![Chapter { start_ms: 0, end_ms: 500, title: "a".into() }, Chapter { start_ms: 1000, end_ms: 2000, title: "b".into() }];
        assert!(validate(&gap, 2.0).is_err());
        // duplicate manual times are reported, not silently merged
        let m = vec![ManualChapter { time: "00:00:01".into(), title: "a".into() }, ManualChapter { time: "0:01".into(), title: "b".into() }];
        assert!(build_chapters(10.0, ChapterMode::Manual, 0.0, &m).unwrap_err().contains("same time"));
        // manual chapter at exactly the end is outside
        let m = vec![ManualChapter { time: "00:00:10".into(), title: "a".into() }];
        assert!(build_chapters(10.0, ChapterMode::Manual, 0.0, &m).is_err());
    }

    #[test]
    fn timestamps_lists_and_source() {
        assert_eq!(parse_timestamp("750.5").unwrap(), 750_500);
        assert_eq!(parse_timestamp("12:30").unwrap(), 750_000);
        assert_eq!(parse_timestamp("1:02:03.250").unwrap(), 3_723_250);
        assert_eq!(parse_timestamp("1h2m3.5s").unwrap(), 3_723_500);
        assert_eq!(parse_timestamp("90s").unwrap(), 90_000);
        assert!(parse_timestamp("abc").is_err());
        assert!(parse_timestamp("5x").is_err());
        assert_eq!(fmt_ms(3_723_250), "01:02:03.250");
        assert_eq!(fmt_ms(750_000), "00:12:30");
        let l = parse_chapter_list("\u{feff}00:00:00 Intro\n\n# comment\n00:12:30 - Boss = fight; #1\n1:00:00.5\tEnd\n0:05\n").unwrap();
        assert_eq!(l.len(), 4);
        assert_eq!(l[1], ManualChapter { time: "00:12:30".into(), title: "Boss = fight; #1".into() });
        assert_eq!(l[2].time, "01:00:00.500");
        assert_eq!(l[3].title, "");
        assert!(parse_chapter_list("00:00:00 a\nnope b\n").unwrap_err().starts_with("Line 2"));
        let src = vec![(0.0, "A".into()), (5.0, "B".into()), (12.0, "C".into())];
        let m = from_source(&src, 4.0, 6.0);
        assert_eq!(m, vec![ManualChapter { time: "00:00:01".into(), title: "B".into() }]);
        let mut s = vec![ManualChapter { time: "00:02:00".into(), title: "b".into() }, ManualChapter { time: "bad".into(), title: "x".into() }, ManualChapter { time: "00:01:00".into(), title: "a".into() }];
        sort_manual(&mut s);
        assert_eq!(s.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(), ["a", "b", "x"]);
    }

    #[test]
    fn chapter_input_and_cleanup() {
        assert!(chapter_input(10.0, ChapterMode::Off, 60.0, &[]).unwrap().is_none());
        let (p, text) = chapter_input(10.0, ChapterMode::Auto, 4.0, &[]).unwrap().unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(TEMP_PREFIX) && name.ends_with(".ffmeta"), "{name}");
        assert_eq!(parse_ffmetadata(&text).unwrap().len(), 3);
        assert!(chapter_input(-1.0, ChapterMode::Auto, 4.0, &[]).is_err());
        let d = std::env::temp_dir().join(format!("stale_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        for f in ["ffenc_chapters_1.ffmeta", "ffenc_chapters_2.txt", "other.ffmeta"] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        assert_eq!(cleanup_stale(&d, std::time::Duration::from_secs(3600)), 0, "fresh files kept");
        assert_eq!(cleanup_stale(&d, std::time::Duration::ZERO), 2);
        assert!(d.join("other.ffmeta").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
