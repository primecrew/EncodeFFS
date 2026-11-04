//! Find ffmpeg/ffprobe. Resolution order:
//! 1. the configured bin folder (settings)
//! 2. an `ffmpeg` folder next to the app exe (`ffmpeg\` or `ffmpeg\bin\`)
//! 3. the app-managed copy (default %LOCALAPPDATA%\FFmpegEncoder-Experimental\ffmpeg\bin, or the folder
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

/// %LOCALAPPDATA%\FFmpegEncoder-Experimental (Linux: $XDG_DATA_HOME or ~/.local/share/FFmpegEncoder).
pub fn local_app_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FFENC_LOCAL_DIR") {
        return PathBuf::from(d); // override for tests
    }
    local_app_dir_named(crate::DATA_DIR_NAME)
}

/// `%LOCALAPPDATA%\<name>` (Linux: $XDG_DATA_HOME/<name> or ~/.local/share/<name>).
pub fn local_app_dir_named(name: &str) -> PathBuf {
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(d).join(name);
    }
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(d).join(name);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h).join(".local/share").join(name);
    }
    std::env::temp_dir().join(name)
}

/// The stable version's app-managed FFmpeg folder (never written by this edition).
pub fn stable_managed_bin() -> PathBuf {
    local_app_dir_named(crate::STABLE_DATA_DIR_NAME).join("ffmpeg").join("bin")
}

/// Root of the app-managed FFmpeg: the folder chosen in Settings, or by default
/// `...\FFmpegEncoder-Experimental\ffmpeg` (see [`super::managed`]).
pub fn managed_root() -> PathBuf {
    super::managed::current_root()
}

/// `...\FFmpegEncoder-Experimental\ffmpeg\bin`
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
}
