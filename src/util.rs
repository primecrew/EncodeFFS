//! Small shared helpers.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Supported video input extensions (same list as the script).
pub const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "avs"];
/// Supported audio inputs for still-image / commentary modes.
pub const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "m4a"];
/// Supported still images.
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg"];

/// Lower-case extension without the dot ("" if none).
pub fn ext_lower(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn has_ext(p: &Path, exts: &[&str]) -> bool {
    let e = ext_lower(p);
    exts.iter().any(|x| *x == e)
}

/// File stem as a String (lossy).
pub fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Format a number like PowerShell's "{0:0.######}" with InvariantCulture:
/// at most 6 decimals, trailing zeros removed, never exponent notation.
pub fn fmt_num(v: f64) -> String {
    let mut s = format!("{:.6}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

/// Replace characters that are unsafe in file names (mirrors the script's
/// `-replace '[^A-Za-z0-9._-]', '_'` used for per-job log names).
pub fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

/// Seconds -> "hh:mm:ss.ff"
pub fn fmt_hms(secs: f64) -> String {
    let secs = secs.max(0.0);
    let total_cs = (secs * 100.0).round() as u64;
    let cs = total_cs % 100;
    let s = (total_cs / 100) % 60;
    let m = (total_cs / 6000) % 60;
    let h = total_cs / 360000;
    format!("{:02}:{:02}:{:02}.{:02}", h, m, s, cs)
}

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A process-unique id for temp file names.
pub fn unique_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}{:x}{:x}", t, std::process::id(), n)
}

/// Render an argv as one human-readable command line (for preview/logging only;
/// the real process is spawned with a Vec of args, never a joined string).
pub fn display_command(program: &Path, args: &[String]) -> String {
    let mut out = quote_for_display(&program.to_string_lossy());
    for a in args {
        out.push(' ');
        out.push_str(&quote_for_display(a));
    }
    out
}

fn quote_for_display(s: &str) -> String {
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || "\"'&|<>^;[]()".contains(c)) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Join a folder and a file name.
pub fn join(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fmt_num_matches_powershell_style() {
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(0.125892541179417), "0.125893");
        assert_eq!(fmt_num(-0.0000001), "0");
        assert_eq!(fmt_num(1234567.0), "1234567");
    }
    #[test]
    fn hms() {
        assert_eq!(fmt_hms(3725.5), "01:02:05.50");
    }
    #[test]
    fn safe() {
        assert_eq!(safe_name("my video (1)"), "my_video__1_");
    }
    #[test]
    fn display_quotes_spaces() {
        let s = display_command(Path::new("ffmpeg"), &["-i".into(), "a b.mp4".into()]);
        assert_eq!(s, "ffmpeg -i \"a b.mp4\"");
    }
}
