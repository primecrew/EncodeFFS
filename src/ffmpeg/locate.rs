//! Find ffmpeg/ffprobe. Resolution order:
//! 1. the configured bin folder (settings)
//! 2. an `ffmpeg` folder next to the app exe (`ffmpeg\` or `ffmpeg\bin\`)
//! 3. the app-managed copy (default %LOCALAPPDATA%\PrimeCrew Software\Encode FFS\ffmpeg\bin, or the folder
//!    chosen in Settings › App-managed FFmpeg location)
//! 4. PATH

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfSource {
    Configured,
    NextToExe,
    Managed,
    SystemPath,
}

impl FfSource {
    pub fn label(self) -> &'static str {
        match self {
            FfSource::Configured => "configured folder",
            FfSource::NextToExe => "folder next to the app",
            FfSource::Managed => "app-managed copy",
            FfSource::SystemPath => "PATH",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FfPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub bin_dir: PathBuf,
    pub source: FfSource,
}

/// Platform executable name ("ffmpeg" -> "ffmpeg.exe" on Windows).
pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Folder contains both ffmpeg and ffprobe.
pub fn bin_dir_ok(dir: &Path) -> bool {
    dir.join(exe("ffmpeg")).is_file() && dir.join(exe("ffprobe")).is_file()
}

/// Validate a user-chosen folder: accepts the bin folder itself or its parent
/// (`…\ffmpeg` containing `bin\`). Returns the folder that holds the exes.
pub fn check_bin_dir(dir: &Path) -> Result<PathBuf, String> {
    if dir.as_os_str().is_empty() {
        return Err("Enter or browse to a folder.".into());
    }
    if !dir.is_dir() {
        return Err(format!("Folder not found: {}", dir.display()));
    }
    if bin_dir_ok(dir) {
        return Ok(dir.to_path_buf());
    }
    if bin_dir_ok(&dir.join("bin")) {
        return Ok(dir.join("bin"));
    }
    let missing: Vec<String> = ["ffmpeg", "ffprobe"].iter().map(|n| exe(n)).filter(|n| !dir.join(n).is_file()).collect();
    Err(format!("Missing in this folder: {}", missing.join(", ")))
}

/// Directory of the running exe.
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// %LOCALAPPDATA% base (Linux: $XDG_DATA_HOME or ~/.local/share). No app folder appended.
pub fn local_data_base() -> PathBuf {
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(d);
    }
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(d);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h).join(".local/share");
    }
    std::env::temp_dir()
}

/// %LOCALAPPDATA%\PrimeCrew Software\Encode FFS (Linux: ~/.local/share/PrimeCrew Software/Encode FFS).
pub fn local_app_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FFENC_LOCAL_DIR") {
        return PathBuf::from(d); // override for tests
    }
    local_data_base().join(crate::data_dir_rel())
}

/// Pre-0.6.0-rc.3 location: %LOCALAPPDATA%\FFmpegEncoder.
pub fn legacy_local_app_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FFENC_LEGACY_LOCAL_DIR") {
        return PathBuf::from(d);
    }
    local_data_base().join(crate::legacy_data_dir_rel())
}

/// `%LOCALAPPDATA%\<name>` (Linux: $XDG_DATA_HOME/<name> or ~/.local/share/<name>).
/// `name` may be a relative path (`PrimeCrew Software/Encode FFS (Codecs)`).
pub fn local_app_dir_named(name: &str) -> PathBuf {
    local_data_base().join(name)
}


/// Root of the app-managed FFmpeg: the folder chosen in Settings, or by default
/// `...\PrimeCrew Software\Encode FFS\ffmpeg` (see [`super::managed`]).
pub fn managed_root() -> PathBuf {
    super::managed::current_root()
}

/// `...\<data dir>\ffmpeg\bin`
pub fn managed_bin() -> PathBuf {
    managed_root().join("bin")
}

/// Is `dir` the app-managed FFmpeg folder? (Compared case-insensitively, ignoring a trailing slash
/// and `\` vs `/`, because the configured path is typed or picked by the user.)
pub fn is_managed_dir(dir: &Path, managed: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().replace('\\', "/").to_lowercase();
        s.trim_end_matches('/').trim_start_matches("//?/").to_string()
    }
    norm(dir) == norm(managed)
}

impl FfPaths {
    /// The copy the app downloaded and manages (found as "app-managed", or configured to that folder).
    pub fn is_app_managed(&self) -> bool {
        self.source == FfSource::Managed || is_managed_dir(&self.bin_dir, &managed_bin())
    }
}

/// Candidate bin folders in resolution order (without PATH).
pub fn candidates(configured: &Path) -> Vec<(PathBuf, FfSource)> {
    let mut v = Vec::new();
    if !configured.as_os_str().is_empty() {
        v.push((configured.to_path_buf(), FfSource::Configured));
    }
    let ed = exe_dir().join("ffmpeg");
    v.push((ed.clone(), FfSource::NextToExe));
    v.push((ed.join("bin"), FfSource::NextToExe));
    v.push((managed_bin(), FfSource::Managed));
    v
}

fn search_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find(|d| bin_dir_ok(d))
}

/// Shorten a path for the “Using: …” line, keeping a short head and the last component
/// (e.g. `D:\...\bin`). Unchanged when it already fits in `max_chars`.
pub fn shorten_path(p: &Path, max_chars: usize) -> String {
    let s = p.to_string_lossy();
    if max_chars == 0 || s.chars().count() <= max_chars {
        return s.into_owned();
    }
    let sep = if s.contains('\\') && !s.contains('/') { '\\' } else { '/' };
    // Split on the display separator so a Windows path still shortens when tests run on Linux.
    let last = s.rsplit(sep).next().unwrap_or("").to_string();
    let head: String = if s.chars().nth(1) == Some(':') {
        s.chars().take(3).collect()
    } else if s.starts_with('/') {
        "/".into()
    } else {
        s.chars().take(2).collect()
    };
    let short = format!("{head}...{sep}{last}");
    if short.chars().count() <= max_chars.max(last.chars().count() + 4) {
        short
    } else {
        format!("...{sep}{last}")
    }
}

impl FfPaths {
    /// Status line for the codec picker, and an optional hover with the full path.
    /// App-managed copies never put the path in the label (it is always in the hover).
    /// The user's own FFmpeg includes a shortened path; the hover is set when it was shortened.
    pub fn using_line(&self, max_path_chars: usize) -> (String, Option<String>) {
        let full = self.bin_dir.display().to_string();
        if self.is_app_managed() {
            ("Using: app-managed FFmpeg".into(), Some(full))
        } else {
            let short = shorten_path(&self.bin_dir, max_path_chars);
            let hover = (short != full).then_some(full);
            (format!("Using: your FFmpeg ({short})"), hover)
        }
    }
}

pub fn locate(configured: &Path) -> Option<FfPaths> {
    let found = candidates(configured)
        .into_iter()
        .find(|(d, _)| bin_dir_ok(d))
        .or_else(|| search_path().map(|d| (d, FfSource::SystemPath)))?;
    let (dir, source) = found;
    Some(FfPaths { ffmpeg: dir.join(exe("ffmpeg")), ffprobe: dir.join(exe("ffprobe")), bin_dir: dir, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_first() {
        let dir = std::env::temp_dir().join(format!("loc_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!bin_dir_ok(&dir));
        std::fs::write(dir.join(exe("ffmpeg")), b"").unwrap();
        std::fs::write(dir.join(exe("ffprobe")), b"").unwrap();
        let f = locate(&dir).unwrap();
        assert_eq!(f.source, FfSource::Configured);
        assert_eq!(f.bin_dir, dir);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn order() {
        let c = candidates(Path::new("D:/x"));
        assert_eq!(c[0].1, FfSource::Configured);
        assert_eq!(c.last().unwrap().1, FfSource::Managed);
    }

    #[test]
    fn check_bin_dir_variants() {
        let d = std::env::temp_dir().join(format!("cbd_{}", crate::util::unique_id()));
        std::fs::create_dir_all(d.join("bin")).unwrap();
        assert!(check_bin_dir(Path::new("")).is_err());
        assert!(check_bin_dir(&d.join("nope")).unwrap_err().contains("not found"));
        std::fs::write(d.join("bin").join(exe("ffmpeg")), b"x").unwrap();
        let e = check_bin_dir(&d.join("bin")).unwrap_err();
        assert!(e.contains(&exe("ffprobe")) && !e.contains(&format!("{}, ", exe("ffmpeg"))), "{e}");
        std::fs::write(d.join("bin").join(exe("ffprobe")), b"x").unwrap();
        assert_eq!(check_bin_dir(&d.join("bin")).unwrap(), d.join("bin"));
        assert_eq!(check_bin_dir(&d).unwrap(), d.join("bin"), "parent folder accepted");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn managed_vs_own_folder() {
        let root = std::env::temp_dir().join(format!("ffenc_mng_{}", crate::util::unique_id()));
        let managed = root.join("FFmpegEncoder").join("ffmpeg").join("bin");
        std::fs::create_dir_all(&managed).unwrap();
        assert!(is_managed_dir(&managed, &managed));
        let typed = PathBuf::from(format!("{}/", managed.to_string_lossy().to_uppercase()));
        if cfg!(windows) {
            assert!(is_managed_dir(&typed, &managed));
        }
        assert!(is_managed_dir(&PathBuf::from(format!("{}/", managed.display())), &managed), "trailing slash");
        assert!(!is_managed_dir(&root.join("my ffmpeg").join("bin"), &managed));
        let own = FfPaths { ffmpeg: "x".into(), ffprobe: "y".into(), bin_dir: root.join("mine"), source: FfSource::Configured };
        assert!(!own.is_app_managed());
        let m = FfPaths { source: FfSource::Managed, ..own.clone() };
        assert!(m.is_app_managed());
        let configured_to_managed = FfPaths { bin_dir: managed_bin(), ..own.clone() };
        assert!(configured_to_managed.is_app_managed());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shorten_path_keeps_short_ones_and_ellipsizes_long_ones() {
        assert_eq!(shorten_path(Path::new(r"D:\ffmpeg\bin"), 40), r"D:\ffmpeg\bin");
        let s = shorten_path(Path::new(r"D:\Users\Public\some long folder\tools\ffmpeg\bin"), 28);
        assert!(s.starts_with(r"D:\"), "{s}");
        assert!(s.ends_with("bin"), "{s}");
        assert!(s.contains("..."), "{s}");
        let u = shorten_path(Path::new("/usr/local/very/long/ffmpeg/bin"), 24);
        assert!(u.contains("..."), "{u}");
        assert!(u.ends_with("bin"), "{u}");
    }

    #[test]
    fn using_line_distinguishes_managed_and_your_ffmpeg() {
        let managed = managed_bin();
        let own = FfPaths {
            ffmpeg: "x".into(),
            ffprobe: "y".into(),
            bin_dir: PathBuf::from(r"D:\Tools\FFmpeg\bin"),
            source: FfSource::Configured,
        };
        let (l, h) = own.using_line(40);
        assert_eq!(l, r"Using: your FFmpeg (D:\Tools\FFmpeg\bin)");
        assert_eq!(h, None, "short path stays in the label");
        let long = FfPaths {
            bin_dir: PathBuf::from(r"D:\Users\Public\some long folder\tools\ffmpeg\bin"),
            ..own.clone()
        };
        let (l, h) = long.using_line(28);
        assert!(l.starts_with("Using: your FFmpeg ("), "{l}");
        assert!(l.contains("..."), "{l}");
        assert_eq!(h.as_deref(), Some(r"D:\Users\Public\some long folder\tools\ffmpeg\bin"));
        let mg = FfPaths { source: FfSource::Managed, bin_dir: managed.clone(), ..own };
        let (l, h) = mg.using_line(40);
        assert_eq!(l, "Using: app-managed FFmpeg");
        assert_eq!(h.as_deref(), Some(managed.display().to_string()).as_deref());
        let path_ff = FfPaths {
            ffmpeg: "x".into(),
            ffprobe: "y".into(),
            bin_dir: PathBuf::from(r"C:\Windows\System32"),
            source: FfSource::SystemPath,
        };
        let (l, _) = path_ff.using_line(40);
        assert_eq!(l, r"Using: your FFmpeg (C:\Windows\System32)");
    }
}
