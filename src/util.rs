//! Small shared helpers.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Supported video input extensions (same list as the script).
pub const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "avs"];
/// Supported audio inputs for still-image / commentary modes.
pub const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "m4a"];
/// Supported still images.
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg"];
/// Video inputs accepted by custom presets.
pub const ANY_VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "avs", "m4v", "webm", "ts", "mts", "m2ts", "flv", "wmv", "mpg", "mpeg"];
/// Audio files accepted by custom presets (inputs and replace / add-track files).
pub const ANY_AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus", "mka", "ac3", "wma"];
/// Subtitle files that can be imported.
pub const SUB_EXTS: &[&str] = &["srt", "ass", "ssa", "vtt", "sup", "mks"];

/// A sibling of `input` with the same stem and one of `exts` (first match in
/// `exts` order), never `input` itself. Used by folder batches to pair e.g.
/// `game.mkv` with `game.wav`.
pub fn find_companion(input: &Path, exts: &[&str]) -> Option<PathBuf> {
    let dir = input.parent()?;
    let st = stem(input).to_lowercase();
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p != input && stem(p).to_lowercase() == st && has_ext(p, exts))
        .collect();
    found.sort_by_key(|p| exts.iter().position(|e| *e == ext_lower(p)).unwrap_or(usize::MAX));
    found.into_iter().next()
}

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

/// Two paths name the same file (case-insensitive on Windows, canonicalised when they exist).
pub fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| {
        let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = c.to_string_lossy().into_owned();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    n(a) == n(b)
}

/// "Natural" order: case-insensitive, digit runs compared by value, so
/// `file2` < `file10` and `Clip 9` < `clip 10`. Ties fall back to a plain comparison.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < ac.len() && j < bc.len() {
        if ac[i].is_ascii_digit() && bc[j].is_ascii_digit() {
            let si = i;
            while i < ac.len() && ac[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < bc.len() && bc[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = ac[si..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let nb: String = bc[sj..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            let (x, y) = (ac[i].to_lowercase().next().unwrap_or(ac[i]), bc[j].to_lowercase().next().unwrap_or(bc[j]));
            if x != y {
                return x.cmp(&y);
            }
            i += 1;
            j += 1;
        }
    }
    (ac.len() - i).cmp(&(bc.len() - j)).then_with(|| a.cmp(b))
}

/// Sort paths by file name in natural order.
pub fn natural_sort_paths(v: &mut [PathBuf]) {
    v.sort_by(|a, b| natural_cmp(&a.file_name().unwrap_or_default().to_string_lossy(), &b.file_name().unwrap_or_default().to_string_lossy()));
}

/// Dropped paths for presets that take files: files are kept in the given order,
/// each folder is replaced by the files directly inside it (natural order).
pub fn expand_dropped(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
            natural_sort_paths(&mut v);
            out.extend(v);
        } else {
            out.push(p.clone());
        }
    }
    out
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

    #[test]
    fn companion() {
        let d = std::env::temp_dir().join(format!("comp_{}", unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        for f in ["Game 1.mkv", "game 1.FLAC", "Game 1.wav", "Game 2.mkv", "Game 1.srt"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert_eq!(find_companion(&d.join("Game 1.mkv"), &["wav", "flac"]).unwrap().file_name().unwrap(), "Game 1.wav");
        assert_eq!(find_companion(&d.join("Game 1.mkv"), &["flac"]).unwrap().file_name().unwrap(), "game 1.FLAC");
        assert!(find_companion(&d.join("Game 2.mkv"), &["wav"]).is_none());
        assert!(find_companion(&d.join("Game 1.mkv"), &["mkv"]).is_none(), "never the input itself");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["file10.mkv", "file2.mkv", "File1.mkv", "file02b.mkv", "clip 9.mp4", "Clip 10.mp4", "a.mkv", "file2.mkv.bak", "ünï 1.mkv"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a.mkv", "clip 9.mp4", "Clip 10.mp4", "File1.mkv", "file2.mkv", "file2.mkv.bak", "file02b.mkv", "file10.mkv", "ünï 1.mkv"]);
        assert_eq!(natural_cmp("x007", "x7"), std::cmp::Ordering::Less, "equal value: deterministic tie-break");
        assert_eq!(natural_cmp("part 1", "part 1"), std::cmp::Ordering::Equal);
        assert_eq!(natural_cmp("99999999999999999999999", "100000000000000000000000"), std::cmp::Ordering::Less, "no integer overflow");
    }

    #[test]
    fn expand_drops() {
        let d = std::env::temp_dir().join(format!("drop_{}", unique_id()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        for f in ["e10.mkv", "e2.mkv", "e1.mkv"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        std::fs::write(d.join("sub/inner.mkv"), b"x").unwrap();
        let single = PathBuf::from("/x/z.mp4");
        let got = expand_dropped(&[single.clone(), d.clone()]);
        let names: Vec<String> = got.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["z.mp4", "e1.mkv", "e2.mkv", "e10.mkv"], "files first as dropped; folder natural-sorted, not recursive");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
