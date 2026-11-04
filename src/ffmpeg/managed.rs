//! Where the app-managed FFmpeg lives (the copy the app downloads, updates and rolls back; not
//! the user's own FFmpeg folder). Default: `%LOCALAPPDATA%\<data dir>\ffmpeg`. The user can choose
//! another folder in Settings (0.6.0-dev.3 / 0.5.2); the managed copy is then `<folder>\bin`, with
//! the rollback backup in `<folder>\bin.previous` and update staging next to them.
//!
//! The location is a process-wide value set from the settings at startup and when it changes, so
//! the updater, rollback, update checks and "app-managed vs own folder" detection all follow it.

use std::path::{Path, PathBuf};

/// Folders that make up a managed copy (moved together).
pub const PARTS: [&str; 2] = ["bin", "bin.previous"];
/// Space needed when there is no copy yet (download + extraction + backup), bytes.
pub const FRESH_NEED: u64 = 600 * 1024 * 1024;
const MARGIN: u64 = 64 * 1024 * 1024;
/// Longest folder path accepted (so `<folder>\bin.previous\ffprobe.exe` stays below MAX_PATH).
pub const MAX_LEN: usize = 200;

#[cfg(not(test))]
static OVERRIDE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

#[cfg(test)]
thread_local! {
    // per test thread, so parallel tests don't see each other's location
    static OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// The default location (`%LOCALAPPDATA%\<data dir>\ffmpeg`).
pub fn default_root() -> PathBuf {
    super::locate::local_app_dir().join("ffmpeg")
}

/// The setting's value as a folder: empty = default; quotes and trailing separators removed.
pub fn resolve_root(setting: &str, default: &Path) -> PathBuf {
    let s = clean(setting);
    if s.is_empty() {
        default.to_path_buf()
    } else {
        PathBuf::from(s)
    }
}

fn clean(s: &str) -> String {
    let s = s.trim().trim_matches('"').trim();
    let mut s = s.to_string();
    // keep "C:\" and "\\" roots intact
    while (s.ends_with('\\') || s.ends_with('/')) && s.len() > 3 {
        s.pop();
    }
    s
}

/// Use the folder from the settings (empty = default) for this process.
pub fn set_location(setting: &str) {
    let v = if clean(setting).is_empty() { None } else { Some(resolve_root(setting, &default_root())) };
    #[cfg(not(test))]
    {
        *OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = v;
    }
    #[cfg(test)]
    OVERRIDE.with(|o| *o.borrow_mut() = v);
}

/// Current managed root (the chosen folder, or the default).
pub fn current_root() -> PathBuf {
    #[cfg(not(test))]
    let v = OVERRIDE.read().unwrap_or_else(|e| e.into_inner()).clone();
    #[cfg(test)]
    let v = OVERRIDE.with(|o| o.borrow().clone());
    v.unwrap_or_else(default_root)
}

/// Is a custom (non-default) location in use?
pub fn is_custom() -> bool {
    !super::locate::is_managed_dir(&current_root(), &default_root())
}

fn norm(s: &str) -> String {
    let mut s = s.replace('/', "\\").to_lowercase();
    while s.ends_with('\\') && s.len() > 3 {
        s.pop();
    }
    s
}

/// `a` is `b` or inside it (text comparison, Windows rules: case-insensitive, \ or /).
pub fn is_within(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    !b.is_empty() && (a == b || a.starts_with(&format!("{}\\", b.trim_end_matches('\\'))))
}

/// Folders a managed copy must never go into.
pub fn system_dirs() -> Vec<String> {
    let mut v: Vec<String> = ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "CommonProgramFiles", "CommonProgramFiles(x86)"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .filter(|s| !s.trim().is_empty())
        .collect();
    v.extend([r"C:\Windows", r"C:\Program Files", r"C:\Program Files (x86)"].map(String::from));
    v
}

/// Context for [`validate_text`]: what the chosen folder must not be.
pub struct Rules {
    /// Current managed root (choosing it again is "no change"; a folder inside it is refused).
    pub current: String,
    /// The stable edition's data folder (never written by the experimental edition); empty = none.
    pub protected: Vec<String>,
    pub system: Vec<String>,
}

const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Check a typed/picked folder without touching the disk (Windows path rules; on other systems
/// absolute `/…` paths are accepted too, for tests). Returns the cleaned folder.
pub fn validate_text(input: &str, r: &Rules) -> Result<String, String> {
    let s = clean(input);
    if s.is_empty() {
        return Err("Enter or browse to a folder.".into());
    }
    if s.chars().count() > MAX_LEN {
        return Err(format!("That path is too long (over {MAX_LEN} characters). Choose a shorter folder."));
    }
    let b = s.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    let unc = s.starts_with(r"\\") || s.starts_with("//");
    let unix = !cfg!(windows) && s.starts_with('/') && !unc;
    if !(drive || unc || unix) {
        return Err("Use a full path, for example D:\\Tools\\FFmpeg or \\\\server\\share\\FFmpeg.".into());
    }
    let rest = if drive { &s[3..] } else if unc { &s[2..] } else { &s[1..] };
    if let Some(c) = rest.chars().find(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*' | ':') || c.is_control()) {
        return Err(format!("A folder name can't contain the character {c:?}."));
    }
    if rest.is_empty() {
        return Err("Choose a folder, not the root of a drive or share.".into());
    }
    let parts: Vec<&str> = rest.split(['\\', '/']).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err("The path has an empty folder name (two separators in a row).".into());
    }
    for p in &parts {
        if *p == "." || *p == ".." {
            return Err("Don't use . or .. in the path; choose the folder itself.".into());
        }
        if p.ends_with(' ') || p.ends_with('.') {
            return Err(format!("The folder name \"{p}\" ends with a space or dot, which Windows doesn't allow."));
        }
        let stem = p.split('.').next().unwrap_or("").to_lowercase();
        if RESERVED.contains(&stem.as_str()) {
            return Err(format!("\"{p}\" is a reserved name on Windows."));
        }
    }
    let depth = if unc { parts.len().saturating_sub(2) } else { parts.len() };
    if rest.is_empty() || depth == 0 {
        return Err("Choose a folder, not the root of a drive or share.".into());
    }
    if let Some(sys) = r.system.iter().find(|d| is_within(&s, d)) {
        return Err(format!("That's a Windows system folder ({sys}). Choose a folder of your own."));
    }
    let lower_parts: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
    if lower_parts.iter().any(|p| p == "$recycle.bin" || p == "system volume information") {
        return Err("That's a Windows system folder. Choose a folder of your own.".into());
    }
    if let Some(p) = r.protected.iter().filter(|p| !p.is_empty()).find(|p| is_within(&s, p)) {
        return Err(format!("That folder belongs to the other edition of the app ({p})."));
    }
    if !r.current.is_empty() && norm(&s) == norm(&r.current) {
        return Err("That's already the location.".into());
    }
    if !r.current.is_empty() && is_within(&s, &r.current) {
        return Err("Choose a folder outside the current location (not inside it).".into());
    }
    Ok(s)
}

/// Size of the files of the managed copy at `root` (bin + backup).
pub fn copy_size(root: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| e.metadata().map(|m| if m.is_dir() { walk(&e.path()) } else { m.len() }).unwrap_or(0)).sum())
            .unwrap_or(0)
    }
    PARTS.iter().map(|p| walk(&root.join(p))).sum()
}

/// Bytes needed at the new location: the copy being moved (plus a margin), or room for a download.
pub fn space_needed(moving: Option<u64>) -> u64 {
    match moving {
        Some(n) => n + MARGIN,
        None => FRESH_NEED,
    }
}

/// Free bytes on the volume of `dir` (or its nearest existing parent). None = unknown.
pub fn free_space(dir: &Path) -> Option<u64> {
    let mut d = dir.to_path_buf();
    while !d.exists() {
        d = d.parent()?.to_path_buf();
    }
    free_space_at(&d)
}

#[cfg(windows)]
fn free_space_at(d: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let w: Vec<u16> = d.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free: u64 = 0;
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
fn free_space_at(_d: &Path) -> Option<u64> {
    None
}

/// Checks that need the disk: the folder can be created and written, and has room.
pub fn check_folder(dir: &Path, needed: u64) -> Result<(), String> {
    check_folder_with(dir, needed, free_space(dir))
}

pub fn check_folder_with(dir: &Path, needed: u64, free: Option<u64>) -> Result<(), String> {
    if dir.exists() && !dir.is_dir() {
        return Err(format!("{} is a file, not a folder.", dir.display()));
    }
    let existed = dir.exists();
    if !crate::updater::install::can_write(dir) {
        if !existed {
            let _ = std::fs::remove_dir(dir);
        }
        return Err(format!("Can't create or write files in {}. Choose a folder you can write to.", dir.display()));
    }
    if !existed {
        let _ = std::fs::remove_dir(dir); // created only to test; made again when used
    }
    if let Some(f) = free {
        if f < needed {
            return Err(format!("Not enough free space there: {} MB free, about {} MB needed.", f / 1_048_576, needed / 1_048_576));
        }
    }
    Ok(())
}

/// The new location already holds an FFmpeg copy (bin or backup)?
pub fn has_copy(root: &Path) -> bool {
    super::locate::bin_dir_ok(&root.join("bin"))
}

fn occupied(root: &Path) -> bool {
    PARTS.iter().any(|p| std::fs::read_dir(root.join(p)).map(|mut r| r.next().is_some()).unwrap_or(false))
}

/// Result of a successful move.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveReport {
    pub files: usize,
    pub bytes: u64,
    /// The old copy was deleted (false: it couldn't be; `note` says why).
    pub old_removed: bool,
    pub note: Option<String>,
}

fn copy_tree(from: &Path, to: &Path, files: &mut usize, bytes: &mut u64) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("Cannot create {}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| format!("Cannot read {}: {e}", from.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&src, &dst, files, bytes)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("Cannot copy {} to {}: {e}", src.display(), dst.display()))?;
            // compare the files themselves (the byte count fs::copy returns isn't reliable on every
            // Windows-compatible system, e.g. Wine reports a different number)
            let want = std::fs::metadata(&src).map(|m| m.len()).map_err(|e| format!("Cannot read {}: {e}", src.display()))?;
            let n = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(u64::MAX);
            if n != want {
                return Err(format!("Copy of {} is incomplete.", src.display()));
            }
            *files += 1;
            *bytes += n;
        }
    }
    Ok(())
}

/// Move the managed copy from `old_root` to `new_root`: copy `bin` (and the rollback backup
/// `bin.previous`) to the new folder, check that the copied FFmpeg runs (`verify` gets the new bin
/// folder), then delete the old copy. If anything fails before the old copy is deleted, everything
/// created at the new location is removed again and the old copy is untouched.
pub fn move_copy(old_root: &Path, new_root: &Path, verify: &dyn Fn(&Path) -> Result<(), String>) -> Result<MoveReport, String> {
    if !has_copy(old_root) {
        return Err(format!("There is no app-managed FFmpeg in {} to move.", old_root.join("bin").display()));
    }
    if occupied(new_root) {
        return Err(format!("{} already contains an FFmpeg folder (bin). Choose an empty folder, or switch without moving to use what's there.", new_root.display()));
    }
    let root_existed = new_root.exists();
    std::fs::create_dir_all(new_root).map_err(|e| format!("Cannot create {}: {e}", new_root.display()))?;
    let staging = new_root.join(format!(".ffenc-move-{}", crate::util::unique_id()));
    let mut placed: Vec<PathBuf> = Vec::new();
    let (mut files, mut bytes) = (0usize, 0u64);
    let r = (|| -> Result<(), String> {
        for p in PARTS {
            let src = old_root.join(p);
            if src.is_dir() {
                copy_tree(&src, &staging.join(p), &mut files, &mut bytes)?;
            }
        }
        for p in PARTS {
            let s = staging.join(p);
            if s.is_dir() {
                let dst = new_root.join(p);
                if dst.exists() {
                    std::fs::remove_dir(&dst).map_err(|e| format!("Cannot use {}: {e}", dst.display()))?;
                }
                std::fs::rename(&s, &dst).map_err(|e| format!("Cannot place {}: {e}", dst.display()))?;
                placed.push(dst);
            }
        }
        verify(&new_root.join("bin")).map_err(|e| format!("The copied FFmpeg doesn't run: {e}"))
    })();
    let _ = std::fs::remove_dir_all(&staging);
    if let Err(e) = r {
        for p in &placed {
            let _ = std::fs::remove_dir_all(p);
        }
        if !root_existed {
            let _ = std::fs::remove_dir(new_root);
        }
        return Err(format!("{e} Nothing was changed: the FFmpeg in {} is still used.", old_root.display()));
    }
    let mut errs = Vec::new();
    for p in PARTS {
        let d = old_root.join(p);
        if d.exists() {
            if let Err(e) = std::fs::remove_dir_all(&d) {
                errs.push(format!("{}: {e}", d.display()));
            }
        }
    }
    let _ = std::fs::remove_dir(old_root); // only if empty
    let old_removed = errs.is_empty();
    let note = (!old_removed).then(|| format!("The old copy could not be deleted completely ({}); you can delete it yourself.", errs.join("; ")));
    Ok(MoveReport { files, bytes, old_removed, note })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::locate::{exe, managed_bin, FfPaths, FfSource};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ffenc_mloc_{tag}_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn fake_copy(root: &Path, with_backup: bool) {
        for (dir, tag) in [("bin", "new"), ("bin.previous", "old")] {
            if dir == "bin.previous" && !with_backup {
                continue;
            }
            std::fs::create_dir_all(root.join(dir)).unwrap();
            for n in ["ffmpeg", "ffprobe"] {
                std::fs::write(root.join(dir).join(exe(n)), format!("{n} {tag}")).unwrap();
            }
        }
    }
    fn rules(current: &str) -> Rules {
        Rules { current: current.into(), protected: vec![r"C:\Users\me\AppData\Local\FFmpegEncoder".into()], system: vec![r"C:\Windows".into(), r"C:\Program Files".into(), r"C:\Program Files (x86)".into()] }
    }

    #[test]
    fn path_resolution_and_location_switch() {
        let def = PathBuf::from("/d/default/ffmpeg");
        assert_eq!(resolve_root("", &def), def);
        assert_eq!(resolve_root("   ", &def), def);
        assert_eq!(resolve_root(r#" "D:\My Tools\FFmpeg\" "#, &def), PathBuf::from(r"D:\My Tools\FFmpeg"));
        assert_eq!(resolve_root(r"\\nas\média\ffmpeg\", &def), PathBuf::from(r"\\nas\média\ffmpeg"));
        // default when nothing is set; the updater/locate paths follow a custom location
        set_location("");
        assert_eq!(current_root(), default_root());
        assert!(!is_custom());
        assert_eq!(managed_bin(), default_root().join("bin"));
        let custom = tmp("loc");
        set_location(&custom.to_string_lossy());
        assert!(is_custom());
        assert_eq!(current_root(), custom);
        assert_eq!(managed_bin(), custom.join("bin"));
        assert_eq!(crate::updater::install::backup_dir_for(&managed_bin()), custom.join("bin.previous"), "rollback backup follows");
        set_location("");
        assert_eq!(managed_bin(), default_root().join("bin"));
        std::fs::remove_dir_all(&custom).unwrap();
    }

    #[test]
    fn managed_vs_own_copy_follows_the_location() {
        let custom = tmp("det");
        fake_copy(&custom, false);
        let own = tmp("own");
        fake_copy(&own, false);
        set_location(&custom.to_string_lossy());
        let configured_custom = FfPaths { ffmpeg: "x".into(), ffprobe: "y".into(), bin_dir: custom.join("bin"), source: FfSource::Configured };
        assert!(configured_custom.is_app_managed(), "the copy in the chosen location is app-managed");
        let at_default = FfPaths { bin_dir: default_root().join("bin"), ..configured_custom.clone() };
        assert!(!at_default.is_app_managed(), "the old default folder is no longer the managed one");
        let mine = FfPaths { bin_dir: own.join("bin"), ..configured_custom.clone() };
        assert!(!mine.is_app_managed());
        // locate finds the managed copy at the new location
        let f = crate::ffmpeg::locate::locate(Path::new("")).filter(|f| f.source == FfSource::Managed);
        if let Some(f) = f {
            assert_eq!(f.bin_dir, custom.join("bin"));
        }
        set_location("");
        assert!(at_default.is_app_managed());
        std::fs::remove_dir_all(&custom).unwrap();
        std::fs::remove_dir_all(&own).unwrap();
    }

    #[test]
    fn validation_rules() {
        let r = rules(r"C:\Users\me\AppData\Local\FFmpegEncoder-Experimental\ffmpeg");
        let ok = |s: &str| validate_text(s, &r);
        assert_eq!(ok(r"D:\Tools\FFmpeg").unwrap(), r"D:\Tools\FFmpeg");
        assert_eq!(ok(r"  D:\My Tools\FFmpeg Managed\  ").unwrap(), r"D:\My Tools\FFmpeg Managed", "spaces + trailing slash");
        assert!(ok(r"E:\Видео\ffmpeg").is_ok(), "unicode");
        assert!(ok(r"\\nas\share\apps\ffmpeg").is_ok(), "network (UNC) path");
        assert!(ok(r"D:/Tools/FFmpeg").is_ok(), "forward slashes");
        for (bad, why) in [
            ("", "Enter"),
            (r"Tools\FFmpeg", "full path"),
            (r"D:\", "root"),
            (r"\\nas\share", "root"),
            (r"D:\a<b", "character"),
            (r"D:\a\b:c", "character"),
            (r"D:\a\\b", "empty folder name"),
            (r"D:\a\..\b", ".."),
            (r"D:\dots.", "space or dot"),
            (r"D:\x\CON", "reserved"),
            (r"D:\x\nul.txt", "reserved"),
            (r"C:\Windows\System32\ff", "system folder"),
            (r"c:\program files\ffmpeg", "system folder"),
            (r"D:\$Recycle.Bin\x", "system folder"),
            (r"C:\Users\me\AppData\Local\FFmpegEncoder\ffmpeg", "other edition"),
            (r"c:\users\me\appdata\local\ffmpegencoder-experimental\ffmpeg\", "already"),
            (r"C:\Users\me\AppData\Local\FFmpegEncoder-Experimental\ffmpeg\sub", "outside"),
        ] {
            let e = ok(bad).expect_err(bad);
            assert!(e.contains(why), "{bad:?}: {e}");
        }
        let long = format!(r"D:\{}", "a".repeat(MAX_LEN));
        assert!(ok(&long).unwrap_err().contains("too long"));
        // the other edition's folder name as a prefix of a different folder is fine
        assert!(ok(r"C:\Users\me\AppData\Local\FFmpegEncoderTools").is_ok());
    }

    #[test]
    fn folder_checks_writable_and_space() {
        let d = tmp("chk");
        let target = d.join("new place");
        assert!(check_folder_with(&target, 100, Some(1000)).is_ok());
        assert!(!target.exists(), "the test folder isn't left behind");
        assert!(check_folder_with(&target, 100, None).is_ok(), "unknown free space doesn't block");
        let e = check_folder_with(&target, 700 * 1_048_576, Some(10 * 1_048_576)).unwrap_err();
        assert!(e.contains("Not enough free space") && e.contains("10 MB free"), "{e}");
        std::fs::write(d.join("file"), b"x").unwrap();
        assert!(check_folder_with(&d.join("file"), 1, None).unwrap_err().contains("is a file"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = d.join("ro");
            std::fs::create_dir_all(&ro).unwrap();
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
            let is_root = std::fs::write(ro.join("probe"), b"x").is_ok();
            if !is_root {
                assert!(check_folder_with(&ro.join("x"), 1, None).unwrap_err().contains("Can't create"));
            }
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(space_needed(None), FRESH_NEED);
        assert_eq!(space_needed(Some(100)), 100 + MARGIN);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn move_then_remove_old() {
        let d = tmp("mv");
        let (old, new) = (d.join("old"), d.join("new place ü"));
        fake_copy(&old, true);
        let size = copy_size(&old);
        assert!(size > 0);
        let rep = move_copy(&old, &new, &|bin| if crate::ffmpeg::locate::bin_dir_ok(bin) { Ok(()) } else { Err("missing".into()) }).unwrap();
        assert_eq!((rep.files, rep.bytes, rep.old_removed), (4, size, true));
        assert!(has_copy(&new));
        assert_eq!(std::fs::read_to_string(new.join("bin.previous").join(exe("ffmpeg"))).unwrap(), "ffmpeg old", "rollback backup moved too");
        assert!(!old.exists(), "old copy removed");
        assert!(std::fs::read_dir(&new).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".ffenc-move")), "no staging left");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failed_verify_rolls_back() {
        let d = tmp("rb");
        let (old, new) = (d.join("old"), d.join("new"));
        fake_copy(&old, true);
        let e = move_copy(&old, &new, &|_| Err("exit code 1".into())).unwrap_err();
        assert!(e.contains("doesn't run") && e.contains("Nothing was changed"), "{e}");
        assert!(has_copy(&old) && old.join("bin.previous").is_dir(), "old copy untouched");
        assert!(!new.exists(), "new folder removed again (it didn't exist before)");
        // an existing (empty) destination folder is kept, but nothing is left in it
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("my notes.txt"), b"keep").unwrap();
        assert!(move_copy(&old, &new, &|_| Err("no".into())).is_err());
        let left: Vec<String> = std::fs::read_dir(&new).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["my notes.txt".to_string()]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn refuses_bad_moves() {
        let d = tmp("ref");
        let (old, new) = (d.join("old"), d.join("new"));
        assert!(move_copy(&old, &new, &|_| Ok(())).unwrap_err().contains("no app-managed FFmpeg"));
        fake_copy(&old, false);
        fake_copy(&new, false);
        let e = move_copy(&old, &new, &|_| Ok(())).unwrap_err();
        assert!(e.contains("already contains"), "{e}");
        assert!(has_copy(&old) && has_copy(&new), "nothing touched");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn is_within_rules() {
        assert!(is_within(r"C:\Windows", r"c:\windows\"));
        assert!(is_within(r"C:\Windows\x", r"C:\Windows"));
        assert!(!is_within(r"C:\WindowsApps2", r"C:\Windows"));
        assert!(is_within("D:/a/b", r"D:\a"));
    }
}
