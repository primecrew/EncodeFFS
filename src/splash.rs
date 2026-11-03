//! Splash screen content: built-in defaults (assets/splash.txt + the app icon) that an optional
//! `splash.txt` / `splash.png` next to the exe (or in the settings folder) replaces.

use std::path::PathBuf;

pub const TEXT_FILE: &str = "splash.txt";
pub const IMAGE_FILE: &str = "splash.png";
/// Built-in text (placeholder until the final text is supplied).
pub const DEFAULT_TEXT: &str = include_str!("../assets/splash.txt");
/// How long the splash stays up unless dismissed earlier.
pub const DURATION_MS: u64 = 1800;

#[derive(Debug, Clone, PartialEq)]
pub struct SplashText {
    pub title: String,
    pub tagline: String,
    pub body: Vec<String>,
}

/// Parse splash.txt: `title:` and `tagline:` lines, `#` comments, everything else is body text.
pub fn parse(text: &str) -> SplashText {
    let mut s = SplashText { title: crate::APP_NAME.to_string(), tagline: String::new(), body: vec![] };
    for line in text.trim_start_matches('\u{feff}').lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let lower = l.to_ascii_lowercase();
        if lower.starts_with("title:") {
            let v = l[6..].trim();
            if !v.is_empty() {
                s.title = v.to_string();
            }
        } else if lower.starts_with("tagline:") {
            s.tagline = l[8..].trim().to_string();
        } else {
            s.body.push(l.to_string());
        }
    }
    s
}

/// First existing `name` in `dirs` (exe folder first, then the settings folder).
pub fn find_override(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The folders searched for splash.txt / splash.png, in order.
pub fn override_dirs() -> Vec<PathBuf> {
    let mut v = vec![crate::ffmpeg::locate::exe_dir()];
    if let Some(d) = crate::settings::settings_path().parent() {
        if !v.contains(&d.to_path_buf()) {
            v.push(d.to_path_buf());
        }
    }
    v
}

/// Text to show: the override file when present and readable, else the built-in one.
pub fn load_text(dirs: &[PathBuf]) -> (SplashText, Option<PathBuf>) {
    if let Some(p) = find_override(dirs, TEXT_FILE) {
        if let Ok(t) = std::fs::read_to_string(&p) {
            return (parse(&t), Some(p));
        }
    }
    (parse(DEFAULT_TEXT), None)
}

/// Bytes of the override image, if any (decoding happens in the UI; a broken file falls back).
pub fn load_image_override(dirs: &[PathBuf]) -> Option<(Vec<u8>, PathBuf)> {
    let p = find_override(dirs, IMAGE_FILE)?;
    std::fs::read(&p).ok().map(|b| (b, p))
}

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_and_parsing() {
        let d = parse(DEFAULT_TEXT);
        assert_eq!(d.title, "FFmpeg Encoder");
        assert!(d.tagline.to_lowercase().contains("placeholder"));
        assert!(d.body.is_empty(), "comments are ignored: {:?}", d.body);
        let c = parse("\u{feff}# c\nTitle:  My App \nTAGLINE: Fast & simple\n\nLine one\nLine two\ntitle:\n");
        assert_eq!(c, SplashText { title: "My App".into(), tagline: "Fast & simple".into(), body: vec!["Line one".into(), "Line two".into()] });
        assert_eq!(parse("").title, crate::APP_NAME);
    }

    #[test]
    fn override_files_win_in_order() {
        let root = std::env::temp_dir().join(format!("ffenc_splash_{}", crate::util::unique_id()));
        let (a, b) = (root.join("exe"), root.join("settings"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let dirs = vec![a.clone(), b.clone()];
        assert_eq!(load_text(&dirs).1, None, "built-in when there is no file");
        assert!(load_image_override(&dirs).is_none());
        std::fs::write(b.join(TEXT_FILE), "title: From settings").unwrap();
        assert_eq!(load_text(&dirs).0.title, "From settings");
        std::fs::write(a.join(TEXT_FILE), "title: From exe folder").unwrap();
        let (t, p) = load_text(&dirs);
        assert_eq!((t.title.as_str(), p), ("From exe folder", Some(a.join(TEXT_FILE))));
        std::fs::write(b.join(IMAGE_FILE), b"\x89PNG\r\n\x1a\nxx").unwrap();
        let (bytes, p) = load_image_override(&dirs).unwrap();
        assert!(is_png(&bytes) && p == b.join(IMAGE_FILE));
        assert!(!is_png(b"GIF89a"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
