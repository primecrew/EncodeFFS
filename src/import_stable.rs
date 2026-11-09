//! "Import settings from stable version": copy the stable FFmpeg Encoder's settings and presets
//! into this (slideshow) edition. The stable files are only ever *read*; nothing of the
//! stable version is modified, moved or deleted, and its downloaded FFmpeg is not copied.

use std::path::{Path, PathBuf};

use crate::presets::presets_path_for;
use crate::settings::{stable_settings_candidates, Settings};

/// The stable version's files that were found.
#[derive(Debug, Clone, PartialEq)]
pub struct StableFound {
    pub settings: PathBuf,
    /// Presets file next to it, when there is one.
    pub presets: Option<PathBuf>,
}

/// First existing settings file among `candidates` (portable first, then %APPDATA%).
pub fn find_in(candidates: &[PathBuf]) -> Option<StableFound> {
    let settings = candidates.iter().find(|p| p.is_file())?.clone();
    let presets = Some(presets_path_for(&settings)).filter(|p| p.is_file());
    Some(StableFound { settings, presets })
}

/// Look for the stable version's settings in its usual places.
pub fn find() -> Option<StableFound> {
    find_in(&stable_settings_candidates())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    pub settings_to: PathBuf,
    pub presets_to: Option<PathBuf>,
    /// Plain-language notes for the log / dialog.
    pub notes: Vec<String>,
}

fn same_file(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a) == c(b)
}

/// Copy the stable settings (and presets) to `dest_settings` (this edition's settings path) and the
/// presets file next to it. Overwrites this edition's files; never writes to the stable ones.
pub fn import_into(stable: &StableFound, dest_settings: &Path, stable_managed_bin: &Path) -> Result<ImportReport, String> {
    let dest_presets = presets_path_for(dest_settings);
    if same_file(&stable.settings, dest_settings) || stable.presets.as_deref().is_some_and(|p| same_file(p, &dest_presets)) {
        return Err("The stable version's files are the same files this version uses, so there is nothing to import.".into());
    }
    let text = std::fs::read_to_string(&stable.settings).map_err(|e| format!("Couldn't read {}: {e}", stable.settings.display()))?;
    let mut s = Settings::from_json(&text).ok_or_else(|| format!("The stable settings file {} couldn't be read (not valid settings).", stable.settings.display()))?;
    // read the presets before writing anything, so a failure leaves this edition unchanged
    let presets_text = match &stable.presets {
        Some(p) => Some(std::fs::read(p).map_err(|e| format!("Couldn't read {}: {e}", p.display()))?),
        None => None,
    };
    let mut notes = vec![format!("Imported settings from the stable version ({}).", stable.settings.display())];
    // the stable version may keep its downloaded FFmpeg in a folder of its own choosing (0.5.2)
    let stable_custom_bin = (!s.managed_ffmpeg_dir.trim().is_empty())
        .then(|| crate::ffmpeg::managed::resolve_root(&s.managed_ffmpeg_dir, Path::new("")).join("bin"));
    // this edition keeps its own app-managed FFmpeg location (never the stable version's folder)
    s.managed_ffmpeg_dir = Settings::load_from(dest_settings).managed_ffmpeg_dir;
    let configured = Path::new(s.ffmpeg_bin_path.trim());
    if crate::ffmpeg::locate::is_managed_dir(configured, stable_managed_bin) || stable_custom_bin.as_deref().is_some_and(|b| crate::ffmpeg::locate::is_managed_dir(configured, b)) {
        notes.push("Your FFmpeg folder is the stable version's downloaded copy. This version uses it as your own folder: it never updates it by itself. Help › Check for FFmpeg updates… can download a separate copy for this version.".into());
    }
    s.save_to(dest_settings)?;
    let presets_to = match presets_text {
        Some(bytes) => {
            if let Some(d) = dest_presets.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let tmp = dest_presets.with_extension("json.tmp");
            std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &dest_presets)).map_err(|e| format!("Couldn't write {}: {e}", dest_presets.display()))?;
            notes.push(format!("Imported presets ({}).", stable.presets.as_ref().unwrap().display()));
            Some(dest_presets)
        }
        None => {
            notes.push("The stable version has no presets file; the built-in presets are used.".into());
            None
        }
    };
    Ok(ImportReport { settings_to: dest_settings.to_path_buf(), presets_to, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("ffenc_imp_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn import_copies_and_never_modifies_stable() {
        let root = tmp();
        let stable_dir = root.join("FFmpegEncoder");
        std::fs::create_dir_all(&stable_dir).unwrap();
        let st = stable_dir.join("settings.json");
        let sp = stable_dir.join("presets.json");
        let st_text = r#"{"crf": 19, "auto_check_updates": false, "ffmpeg_bin_path": "C:\\ff\\bin"}"#;
        std::fs::write(&st, st_text).unwrap();
        std::fs::write(&sp, br#"{"version":2,"presets":[]}"#).unwrap();
        let before = (std::fs::read(&st).unwrap(), std::fs::read(&sp).unwrap(), std::fs::metadata(&st).unwrap().modified().unwrap(), std::fs::metadata(&sp).unwrap().modified().unwrap());

        let found = find_in(&[root.join("nope.settings.json"), st.clone()]).unwrap();
        assert_eq!(found, StableFound { settings: st.clone(), presets: Some(sp.clone()) });
        let dest = root.join("FFmpegEncoder-Slideshow").join("settings.json");
        let r = import_into(&found, &dest, &root.join("managed/bin")).unwrap();
        assert_eq!(r.presets_to, Some(root.join("FFmpegEncoder-Slideshow").join("presets.json")));
        let imported = Settings::load_from(&dest);
        assert_eq!(imported.crf, 19);
        assert!(imported.never_check_updates, "update-check choice carried over");
        assert_eq!(std::fs::read(r.presets_to.unwrap()).unwrap(), before.1);
        // stable files untouched (content and time)
        let after = (std::fs::read(&st).unwrap(), std::fs::read(&sp).unwrap(), std::fs::metadata(&st).unwrap().modified().unwrap(), std::fs::metadata(&sp).unwrap().modified().unwrap());
        assert_eq!(before, after);
        assert_eq!(std::fs::read_dir(&stable_dir).unwrap().count(), 2, "nothing added to the stable folder");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn import_edge_cases() {
        let root = tmp();
        assert_eq!(find_in(&[root.join("a.json")]), None, "no stable version");
        // portable stable pair next to the exe
        let st = root.join("FFmpegEncoder.settings.json");
        std::fs::write(&st, r#"{"ffmpeg_bin_path": "/stable/managed/bin"}"#).unwrap();
        let found = find_in(&[st.clone()]).unwrap();
        assert_eq!(found.presets, None);
        let dest = root.join("FFmpegEncoder-Slideshow.settings.json");
        let r = import_into(&found, &dest, Path::new("/stable/managed/bin")).unwrap();
        // the stable version's own managed-location setting isn't taken over; a custom stable
        // managed folder configured as FFmpeg folder is recognised as the stable copy too
        std::fs::write(&dest, r#"{"managed_ffmpeg_dir": "/mine/ffmpeg"}"#).unwrap();
        std::fs::write(&st, r#"{"ffmpeg_bin_path": "/stable/custom/bin", "managed_ffmpeg_dir": "/stable/custom"}"#).unwrap();
        let r2 = import_into(&found, &dest, Path::new("/stable/managed/bin")).unwrap();
        assert!(r2.notes.iter().any(|n| n.contains("stable version's downloaded copy")), "{:?}", r2.notes);
        assert_eq!(Settings::load_from(&dest).managed_ffmpeg_dir, "/mine/ffmpeg");
        assert!(r.notes.iter().any(|n| n.contains("never updates it")), "{:?}", r.notes);
        assert_eq!(r.presets_to, None);
        // same file: refused
        assert!(import_into(&StableFound { settings: dest.clone(), presets: None }, &dest, Path::new("/m")).is_err());
        // broken stable settings: error, nothing written
        let bad = root.join("bad").join("settings.json");
        std::fs::create_dir_all(bad.parent().unwrap()).unwrap();
        std::fs::write(&bad, "not json").unwrap();
        let dest2 = root.join("exp2").join("settings.json");
        assert!(import_into(&StableFound { settings: bad, presets: None }, &dest2, Path::new("/m")).is_err());
        assert!(!dest2.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn slideshow_data_is_separate_from_stable() {
        use crate::ffmpeg::locate::{local_app_dir_named, managed_bin, stable_managed_bin};
        use crate::settings::*;
        assert_ne!(crate::DATA_DIR_NAME, crate::STABLE_DATA_DIR_NAME);
        assert_eq!(PORTABLE_FILE, format!("{}.settings.json", crate::FILE_STEM));
        assert_eq!(crate::presets::PORTABLE_PRESETS_FILE, format!("{}.presets.json", crate::FILE_STEM));
        assert_ne!(PORTABLE_FILE, STABLE_PORTABLE_FILE);
        assert_ne!(crate::presets::presets_path_for(Path::new(PORTABLE_FILE)), crate::presets::presets_path_for(Path::new(STABLE_PORTABLE_FILE)));
        // folders
        assert!(roaming_dir().ends_with(crate::DATA_DIR_NAME));
        assert_ne!(roaming_dir(), roaming_dir_named(crate::STABLE_DATA_DIR_NAME));
        assert!(!settings_path().starts_with(roaming_dir_named(crate::STABLE_DATA_DIR_NAME)));
        for c in stable_settings_candidates() {
            assert_ne!(settings_path(), c, "never the stable settings file");
            assert_ne!(crate::presets::presets_path(), presets_path_for(&c), "never the stable presets file");
        }
        if std::env::var_os("FFENC_LOCAL_DIR").is_none() {
            assert!(!managed_bin().starts_with(local_app_dir_named(crate::STABLE_DATA_DIR_NAME)), "own downloaded FFmpeg");
        }
        assert_ne!(managed_bin(), stable_managed_bin());
        assert!(logs_dir().starts_with(settings_path().parent().unwrap()));
        assert!(crate::full_title().ends_with("(Slideshow)"));
    }
}
