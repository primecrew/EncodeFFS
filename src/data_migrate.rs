//! One-time copy of installed/non-portable data from the pre-rebrand folders
//! (`%APPDATA%\%LOCALAPPDATA%\FFmpegEncoder`) into `PrimeCrew Software\Encode FFS`.
//!
//! The old folders are never deleted or modified. Portable mode is unchanged
//! (settings stay next to the exe). The user's own FFmpeg folder is never touched.

use std::path::Path;

use crate::ffmpeg::managed;
use crate::settings;

/// Plain one-line result of a completed copy (for the app log).
pub struct Report {
    pub line: String,
    pub files: usize,
    pub copied_ffmpeg: bool,
}

/// Run the migration when this is an installed or other non-portable copy, the new
/// data folder has no settings, and the old folder does. Safe to call more than once.
pub fn migrate_if_needed() -> Result<Option<Report>, String> {
    let exe = crate::ffmpeg::locate::exe_dir();
    if uses_portable_settings(&exe, &settings::roaming_dir()) {
        return Ok(None);
    }
    migrate(
        &settings::roaming_dir(),
        &settings::legacy_roaming_dir(),
        &crate::ffmpeg::locate::local_app_dir(),
        &crate::ffmpeg::locate::legacy_local_app_dir(),
    )
}

/// Settings would be stored next to the exe (portable), not in %APPDATA%.
pub fn uses_portable_settings(exe_dir: &Path, roaming: &Path) -> bool {
    settings::settings_path_in(exe_dir, roaming) != roaming.join("settings.json")
}

/// Copy `old_roaming` → `new_roaming` (and the default app-managed FFmpeg when it
/// lives under `old_local`) if the new folder has no `settings.json` and the old one does.
/// Never overwrites existing destination files; never deletes or changes the source.
pub fn migrate(
    new_roaming: &Path,
    old_roaming: &Path,
    new_local: &Path,
    old_local: &Path,
) -> Result<Option<Report>, String> {
    let new_settings = new_roaming.join("settings.json");
    let old_settings = old_roaming.join("settings.json");
    if new_settings.is_file() {
        return Ok(None);
    }
    if !old_settings.is_file() {
        return Ok(None);
    }

    let mut files = 0usize;
    copy_tree_no_overwrite(old_roaming, new_roaming, &mut files)?;
    if !new_settings.is_file() {
        return Err(format!(
            "Could not copy settings from {} to {}.",
            old_settings.display(),
            new_settings.display()
        ));
    }
    // source must still be intact
    if !old_settings.is_file() {
        return Err(format!("The previous settings file {} is missing after the copy.", old_settings.display()));
    }

    let copied_ffmpeg = copy_managed_ffmpeg_if_needed(&new_settings, new_local, old_local, &mut files)?;

    let line = if copied_ffmpeg {
        format!(
            "Copied settings, presets and the app-managed FFmpeg from {} to {}. The previous folder was left as a backup.",
            old_roaming.display(),
            new_roaming.display()
        )
    } else {
        format!(
            "Copied settings and presets from {} to {}. The previous folder was left as a backup.",
            old_roaming.display(),
            new_roaming.display()
        )
    };
    Ok(Some(Report { line, files, copied_ffmpeg }))
}

fn copy_managed_ffmpeg_if_needed(
    new_settings: &Path,
    new_local: &Path,
    old_local: &Path,
    files: &mut usize,
) -> Result<bool, String> {
    let old_default = old_local.join("ffmpeg");
    let new_default = new_local.join("ffmpeg");
    if !managed::has_copy(&old_default) {
        return Ok(false);
    }
    let setting = read_managed_dir_setting(new_settings).unwrap_or_default();
    let custom = !setting.trim().is_empty();
    if custom {
        let resolved = managed::resolve_root(&setting, &old_default);
        if !is_same_or_within(&resolved, &old_default) && !is_same_or_within(&resolved, old_local) {
            // a folder the user chose elsewhere: leave it, keep the setting as-is
            return Ok(false);
        }
        // the setting pointed at the old default root: copy to the new default and
        // clear the setting so the app uses the new default (the old copy stays put)
        if managed::has_copy(&new_default) {
            clear_managed_dir_setting(new_settings)?;
            return Ok(false);
        }
        copy_tree_no_overwrite(&old_default, &new_default, files)?;
        if !managed::has_copy(&new_default) {
            return Err(format!("Could not copy the app-managed FFmpeg to {}.", new_default.display()));
        }
        if !managed::has_copy(&old_default) {
            return Err("The previous app-managed FFmpeg is missing after the copy.".into());
        }
        clear_managed_dir_setting(new_settings)?;
        return Ok(true);
    }
    if managed::has_copy(&new_default) {
        return Ok(false);
    }
    copy_tree_no_overwrite(&old_default, &new_default, files)?;
    if !managed::has_copy(&new_default) {
        return Err(format!("Could not copy the app-managed FFmpeg to {}.", new_default.display()));
    }
    if !managed::has_copy(&old_default) {
        return Err("The previous app-managed FFmpeg is missing after the copy.".into());
    }
    Ok(true)
}

fn read_managed_dir_setting(settings: &Path) -> Option<String> {
    let text = std::fs::read_to_string(settings).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("managed_ffmpeg_dir")?.as_str().map(|s| s.to_string())
}

fn clear_managed_dir_setting(settings: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(settings).map_err(|e| e.to_string())?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("managed_ffmpeg_dir".into(), serde_json::Value::String(String::new()));
    }
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(settings, out).map_err(|e| format!("Cannot update {}: {e}", settings.display()))
}

fn is_same_or_within(path: &Path, root: &Path) -> bool {
    let a = norm(path);
    let b = norm(root);
    a == b || a.starts_with(&(b.clone() + "/"))
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}

/// Recursively copy files and folders. Existing destination files are skipped.
/// The source is never deleted or written. Symlinks are not followed.
fn copy_tree_no_overwrite(from: &Path, to: &Path, files: &mut usize) -> Result<(), String> {
    let meta = match std::fs::symlink_metadata(from) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Cannot read {}: {e}", from.display())),
    };
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        if to.exists() {
            return Ok(());
        }
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("Cannot create {}: {e}", d.display()))?;
        }
        std::fs::copy(from, to).map_err(|e| format!("Cannot copy {} to {}: {e}", from.display(), to.display()))?;
        *files += 1;
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| format!("Cannot create {}: {e}", to.display()))?;
        for ent in std::fs::read_dir(from).map_err(|e| format!("Cannot read {}: {e}", from.display()))? {
            let ent = ent.map_err(|e| format!("Cannot read {}: {e}", from.display()))?;
            copy_tree_no_overwrite(&ent.path(), &to.join(ent.file_name()), files)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::settings::{Settings, INSTALLED_MARKER};

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("ffenc_dm_{}", crate::util::unique_id()))
    }

    fn write_old(roam: &Path, local: &Path, with_ffmpeg: bool, managed_setting: &str) {
        std::fs::create_dir_all(roam.join("logs")).unwrap();
        let mut s = Settings::default();
        s.crf = 18;
        s.managed_ffmpeg_dir = managed_setting.into();
        s.save_to(&roam.join("settings.json")).unwrap();
        std::fs::write(roam.join("presets.json"), r#"{"presets":[]}"#).unwrap();
        std::fs::write(roam.join("layout.json"), r#"{"version":1}"#).unwrap();
        std::fs::write(roam.join("slideshow-ui.json"), r#"{}"#).unwrap();
        std::fs::write(roam.join("logs").join("job-history.log"), "hist").unwrap();
        if with_ffmpeg {
            let bin = local.join("ffmpeg").join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join(crate::ffmpeg::locate::exe("ffmpeg")), b"ff").unwrap();
            std::fs::write(bin.join(crate::ffmpeg::locate::exe("ffprobe")), b"fp").unwrap();
        }
    }

    #[test]
    fn copies_settings_history_and_default_ffmpeg_and_leaves_the_old_folder() {
        let r = root();
        let (old_r, new_r) = (r.join("old_roam"), r.join("new_roam"));
        let (old_l, new_l) = (r.join("old_local"), r.join("new_local"));
        write_old(&old_r, &old_l, true, "");
        let old_settings = std::fs::read(old_r.join("settings.json")).unwrap();

        let report = migrate(&new_r, &old_r, &new_l, &old_l).unwrap().expect("should copy");
        assert!(report.files >= 5, "{}", report.files);
        assert!(report.copied_ffmpeg);
        assert!(report.line.contains("left as a backup"));

        assert_eq!(std::fs::read(old_r.join("settings.json")).unwrap(), old_settings);
        assert!(old_r.join("presets.json").is_file());
        assert!(old_l.join("ffmpeg").join("bin").join(crate::ffmpeg::locate::exe("ffmpeg")).is_file(), "old FFmpeg left intact");
        assert!(new_r.join("settings.json").is_file());
        assert!(new_r.join("presets.json").is_file());
        assert!(new_r.join("layout.json").is_file());
        assert!(new_r.join("slideshow-ui.json").is_file());
        assert_eq!(std::fs::read_to_string(new_r.join("logs").join("job-history.log")).unwrap(), "hist");
        assert!(new_l.join("ffmpeg").join("bin").join(crate::ffmpeg::locate::exe("ffmpeg")).is_file());
        assert_eq!(Settings::load_from(&new_r.join("settings.json")).crf, 18);
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn does_not_overwrite_if_the_new_folder_already_has_settings() {
        let r = root();
        let (old_r, new_r) = (r.join("old_roam"), r.join("new_roam"));
        let (old_l, new_l) = (r.join("old_local"), r.join("new_local"));
        write_old(&old_r, &old_l, true, "");
        std::fs::create_dir_all(&new_r).unwrap();
        std::fs::write(new_r.join("settings.json"), r#"{"crf": 30}"#).unwrap();

        assert!(migrate(&new_r, &old_r, &new_l, &old_l).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(new_r.join("settings.json")).unwrap(), r#"{"crf": 30}"#);
        assert!(!new_r.join("presets.json").exists());
        assert!(!new_l.join("ffmpeg").exists());
        assert!(old_r.join("settings.json").is_file());
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn custom_managed_location_is_left_where_it_is() {
        let r = root();
        let (old_r, new_r) = (r.join("old_roam"), r.join("new_roam"));
        let (old_l, new_l) = (r.join("old_local"), r.join("new_local"));
        let custom = r.join("my_ffmpeg");
        std::fs::create_dir_all(custom.join("bin")).unwrap();
        std::fs::write(custom.join("bin").join(crate::ffmpeg::locate::exe("ffmpeg")), b"ff").unwrap();
        std::fs::write(custom.join("bin").join(crate::ffmpeg::locate::exe("ffprobe")), b"fp").unwrap();
        write_old(&old_r, &old_l, false, custom.to_str().unwrap());

        let report = migrate(&new_r, &old_r, &new_l, &old_l).unwrap().expect("settings copied");
        assert!(!report.copied_ffmpeg);
        assert!(!new_l.join("ffmpeg").exists(), "must not invent a default copy");
        let s = Settings::load_from(&new_r.join("settings.json"));
        assert_eq!(s.managed_ffmpeg_dir, custom.to_str().unwrap());
        assert!(custom.join("bin").join(crate::ffmpeg::locate::exe("ffmpeg")).is_file());
        std::fs::remove_dir_all(&r).ok();
    }

    #[test]
    fn portable_mode_is_detected_so_the_app_will_skip_roaming_migration() {
        let r = root();
        let exe = r.join("app");
        let roam = r.join("roam");
        std::fs::create_dir_all(&exe).unwrap();
        assert!(uses_portable_settings(&exe, &roam), "writable exe folder, no marker");
        std::fs::write(exe.join(INSTALLED_MARKER), b"").unwrap();
        assert!(!uses_portable_settings(&exe, &roam), "installed marker forces roaming");
        std::fs::remove_dir_all(&r).ok();
    }
}
