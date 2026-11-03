//! Persisted settings (JSON next to the exe for portability, falling back to
//! %APPDATA%\FFmpegEncoder when the exe folder is not writable).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cmd::common::CompressorParams;
use crate::ffmpeg::locate::exe_dir;
use crate::updater::{UpdateChannel, UpdateSource};

pub const PORTABLE_FILE: &str = "FFmpegEncoder.settings.json";

/// Colour theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    /// Follow Windows' app mode (dark/light); dark when it can't be read.
    #[default]
    System,
    Dark,
    Light,
    /// Black background, white text, yellow accent, thick focus outlines.
    HighContrast,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::HighContrast];
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System (follow Windows)",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
            ThemeChoice::HighContrast => "High contrast",
        }
    }
}

/// Text / UI size steps offered in menus and by Ctrl +/-.
pub const SCALE_STEPS: [f32; 10] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
pub const SCALE_MIN: f32 = 0.75;
pub const SCALE_MAX: f32 = 3.0;
pub const MONO_MIN: f32 = 0.8;
pub const MONO_MAX: f32 = 2.0;

/// Look & feel. Applied live; scale applies to everything (text, buttons, icons, rows, popups).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// Overall UI scale (1.0 = 100 %).
    pub scale: f32,
    /// Extra factor for monospace text (commands, logs, chapter text), on top of `scale`.
    pub mono_scale: f32,
    /// Bold interface text (Segoe UI Bold when available).
    pub bold: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { theme: ThemeChoice::System, scale: 1.0, mono_scale: 1.0, bold: false }
    }
}

/// Clamp to the allowed range; NaN/inf become the default.
pub fn clamp_scale(v: f32, min: f32, max: f32) -> f32 {
    if v.is_finite() {
        // keep two decimals so steps compare exactly after a round trip
        ((v.clamp(min, max)) * 100.0).round() / 100.0
    } else {
        1.0
    }
}

/// Next / previous entry of [`SCALE_STEPS`] from an arbitrary current value.
pub fn step_scale(cur: f32, up: bool) -> f32 {
    let cur = clamp_scale(cur, SCALE_MIN, SCALE_MAX);
    if up {
        SCALE_STEPS.iter().copied().find(|s| *s > cur + 0.001).unwrap_or(SCALE_MAX)
    } else {
        SCALE_STEPS.iter().rev().copied().find(|s| *s < cur - 0.001).unwrap_or(SCALE_MIN)
    }
}

impl Appearance {
    pub fn sanitized(mut self) -> Self {
        self.scale = clamp_scale(self.scale, SCALE_MIN, SCALE_MAX);
        self.mono_scale = clamp_scale(self.mono_scale, MONO_MIN, MONO_MAX);
        self
    }
}

/// A broken "appearance" value must not throw away the rest of the settings file.
fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Appearance, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<Appearance>(v).unwrap_or_default().sanitized())
}

/// Unknown/garbage values fall back to the default instead of failing the whole file.
fn lenient_default<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<T>(v).unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ---- defaults from the script
    pub ffmpeg_bin_path: String,
    pub input_folder: String,
    pub output_folder: String,
    pub crf: u32,
    pub audio_bitrate_k: u32,
    pub chapter_interval_min: f64,
    pub preset: String,
    pub trim_start: f64,
    pub trim_end: f64,
    pub gain_db: f64,
    pub compressor: CompressorParams,
    pub commentary_name: String,
    pub fallback_sample_rate: u32,
    // ---- app behaviour
    pub update_source: UpdateSource,
    pub update_channel: UpdateChannel,
    /// Legacy mirror of `!never_check_updates`, still written so older versions keep the same behaviour.
    pub auto_check_updates: bool,
    /// "Turn off Update Check for FFMPEG" (0.5.1): never look for FFmpeg updates automatically.
    /// Off for new users (checks stay on); files from older versions get `!auto_check_updates`,
    /// so their behaviour doesn't change. Help › Check for FFmpeg updates still works.
    pub never_check_updates: bool,
    /// Unix seconds of the last automatic update check.
    pub last_update_check: u64,
    /// Keep ffmpeg_error*.log even when the job succeeds.
    pub keep_logs_on_success: bool,
    /// Unsaved form state per preset id (edited values and/or the files of the current run).
    pub drafts: BTreeMap<String, serde_json::Value>,
    /// Id of the preset that was open last.
    pub last_preset: String,
    /// Right-hand "Source" panel (ffprobe summary of the selected input).
    pub show_source_panel: bool,
    /// Terminal syntax for "Copy command" in the run confirmation.
    pub copy_shell: crate::cmdline::Shell,
    /// Theme, text size, bold (0.3.2; older files get the defaults).
    #[serde(deserialize_with = "lenient")]
    pub appearance: Appearance,
    /// What to do when an output file already exists (0.3.3).
    #[serde(deserialize_with = "lenient_default")]
    pub when_exists: crate::output_guard::WhenExists,
    /// Keep chapter/list files and partial outputs and log their paths (debugging, 0.3.3).
    pub keep_temp_files: bool,
    /// Audio & subtitles: added tracks may differ from the video by this many seconds before a warning (0.4).
    pub duration_tolerance_s: f64,
    /// Legacy (0.1.x): last-used values per mode; migrated into presets on first start of 0.2.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ffmpeg_bin_path: r"C:\ffmpeg\bin".into(),
            input_folder: r"D:\Videos\Inbox".into(),
            output_folder: r"D:\Videos\Out".into(),
            crf: 22,
            audio_bitrate_k: 192,
            chapter_interval_min: 5.0,
            preset: "slow".into(),
            trim_start: 0.0,
            trim_end: 0.0,
            gain_db: 0.0,
            compressor: CompressorParams::default(),
            commentary_name: "Commentary Track".into(),
            fallback_sample_rate: 48000,
            update_source: UpdateSource::BtbN,
            update_channel: UpdateChannel::Stable,
            auto_check_updates: true,
            never_check_updates: false,
            last_update_check: 0,
            keep_logs_on_success: false,
            drafts: BTreeMap::new(),
            last_preset: String::new(),
            show_source_panel: true,
            copy_shell: crate::cmdline::Shell::PowerShell,
            appearance: Appearance::default(),
            when_exists: crate::output_guard::WhenExists::Ask,
            keep_temp_files: false,
            duration_tolerance_s: 1.5,
            modes: BTreeMap::new(),
            last_mode: String::new(),
        }
    }
}

fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ffenc_write_test_{}", std::process::id()));
    match std::fs::write(&probe, b"x") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// %APPDATA%\FFmpegEncoder (Linux: ~/.config/FFmpegEncoder)
pub fn roaming_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("APPDATA") {
        return PathBuf::from(d).join("FFmpegEncoder");
    }
    if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(d).join("FFmpegEncoder");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h).join(".config/FFmpegEncoder");
    }
    std::env::temp_dir().join("FFmpegEncoder")
}

/// Where settings live: next to the exe if that folder is writable (portable),
/// otherwise %APPDATA%\FFmpegEncoder\settings.json.
pub fn settings_path() -> PathBuf {
    let ed = exe_dir();
    let portable = ed.join(PORTABLE_FILE);
    if (portable.exists() && std::fs::OpenOptions::new().append(true).open(&portable).is_ok()) || dir_writable(&ed) {
        return portable;
    }
    roaming_dir().join("settings.json")
}

/// Folder for the app log (next to the settings file).
pub fn logs_dir() -> PathBuf {
    settings_path().parent().map(|p| p.join("logs")).unwrap_or_else(|| roaming_dir().join("logs"))
}

/// What started an FFmpeg update check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckTrigger {
    /// The once-a-day check at startup (and the banner it may show).
    Automatic,
    /// The user chose Help › Check for FFmpeg updates… (or a download button).
    Manual,
}

/// Which FFmpeg the app is using, as far as update checks are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegInUse {
    NotFound,
    /// The copy the app downloaded (Help › Check for FFmpeg updates). Automatic checks apply.
    AppManaged,
    /// The user's own folder (configured, next to the app, or PATH). No automatic checks.
    OwnFolder,
}

/// Seconds between automatic checks.
pub const AUTO_CHECK_INTERVAL_S: u64 = 86_400;

impl Settings {
    /// Parse a settings file's text. Files written before 0.5.1 have no `never_check_updates`;
    /// they keep their old behaviour (`auto_check_updates`, which defaulted to on).
    pub fn from_json(text: &str) -> Option<Settings> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let mut s: Settings = serde_json::from_value(v.clone()).ok()?;
        if v.get("never_check_updates").is_none() {
            let auto = v.get("auto_check_updates").and_then(|a| a.as_bool()).unwrap_or(true);
            s.never_check_updates = !auto;
        }
        s.auto_check_updates = !s.never_check_updates;
        s.appearance = s.appearance.sanitized();
        Some(s)
    }

    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|t| Self::from_json(&t)).unwrap_or_default()
    }

    /// May an update check run now? Manual checks always may. Automatic ones only when the
    /// app-managed FFmpeg is in use (never for the user's own folder), "Turn off Update Check for
    /// FFMPEG" is unchecked and the last check is a day old.
    pub fn may_check_updates(&self, trigger: CheckTrigger, now_unix: u64, ffmpeg: FfmpegInUse) -> bool {
        match trigger {
            CheckTrigger::Manual => true,
            CheckTrigger::Automatic => {
                ffmpeg == FfmpegInUse::AppManaged && !self.never_check_updates && now_unix.saturating_sub(self.last_update_check) > AUTO_CHECK_INTERVAL_S
            }
        }
    }

    /// Does the "Turn off Update Check for FFMPEG" option apply? Not with the user's own FFmpeg folder.
    pub fn update_check_option_applies(ffmpeg: FfmpegInUse) -> bool {
        ffmpeg != FfmpegInUse::OwnFolder
    }

    pub fn load() -> Settings {
        Self::load_from(&settings_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let mut out = self.clone();
        out.auto_check_updates = !out.never_check_updates;
        let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
        // write-then-rename so a crash never leaves a truncated settings file
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("Could not save settings to {}: {e}", path.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("Could not save settings to {}: {e}", path.display()))
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&settings_path())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.crf > 51 {
            return Err("CRF must be 0..51".into());
        }
        if self.audio_bitrate_k == 0 || self.audio_bitrate_k > 1024 {
            return Err("Audio bitrate must be 1..1024 kbps".into());
        }
        if !(-30.0..=30.0).contains(&self.gain_db) {
            return Err("Gain must be -30..+30 dB".into());
        }
        if self.trim_start < 0.0 || self.trim_end < 0.0 {
            return Err("Trim values must be >= 0".into());
        }
        if self.chapter_interval_min <= 0.0 {
            return Err("Chapter interval must be > 0".into());
        }
        if self.fallback_sample_rate == 0 {
            return Err("Fallback sample rate must be > 0".into());
        }
        if !crate::cmd::common::PRESETS.contains(&self.preset.as_str()) {
            return Err(format!("Unknown preset {}", self.preset));
        }
        self.compressor.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_match_script() {
        let s = Settings::default();
        assert_eq!(s.ffmpeg_bin_path, r"C:\ffmpeg\bin");
        assert_eq!(s.input_folder, r"D:\Videos\Inbox");
        assert_eq!(s.output_folder, r"D:\Videos\Out");
        assert_eq!((s.crf, s.audio_bitrate_k, s.preset.as_str()), (22, 192, "slow"));
        assert_eq!(s.chapter_interval_min, 5.0);
        assert_eq!(s.compressor, CompressorParams::default());
        assert_eq!(s.commentary_name, "Commentary Track");
        assert_eq!(s.fallback_sample_rate, 48000);
        assert!(s.validate().is_ok());
    }
    #[test]
    fn roundtrip_and_partial_json() {
        let dir = std::env::temp_dir().join(format!("set_{}", crate::util::unique_id()));
        let p = dir.join("s.json");
        let mut s = Settings::default();
        s.crf = 18;
        s.modes.insert("reencode".into(), serde_json::json!({"x": 1}));
        s.save_to(&p).unwrap();
        assert_eq!(Settings::load_from(&p), s);
        // missing keys fall back to defaults
        std::fs::write(&p, r#"{"crf": 30}"#).unwrap();
        let l = Settings::load_from(&p);
        assert_eq!(l.crf, 30);
        assert_eq!(l.audio_bitrate_k, 192);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn appearance_migration_and_clamping() {
        let dir = std::env::temp_dir().join(format!("ffenc_set_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.json");
        // a 0.3.1 file: no "appearance" key
        std::fs::write(&p, r#"{"crf": 20, "copy_shell": "Cmd", "show_source_panel": false}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!((s.crf, s.show_source_panel), (20, false));
        assert_eq!(s.appearance, Appearance::default());
        // out of range / garbage values are clamped or defaulted, the rest of the file survives
        std::fs::write(&p, r#"{"crf": 21, "appearance": {"theme": "Light", "scale": 9.0, "mono_scale": 0.1, "bold": true}}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!(s.crf, 21);
        assert_eq!(s.appearance, Appearance { theme: ThemeChoice::Light, scale: 3.0, mono_scale: 0.8, bold: true });
        std::fs::write(&p, r#"{"crf": 22, "appearance": {"theme": "Neon", "scale": "big"}}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!((s.crf, s.appearance.clone()), (22, Appearance::default()));
        std::fs::write(&p, r#"{"crf": 23, "appearance": "dark"}"#).unwrap();
        assert_eq!(Settings::load_from(&p).crf, 23);
        // round trip
        let mut s = Settings::default();
        s.appearance = Appearance { theme: ThemeChoice::HighContrast, scale: 2.5, mono_scale: 1.25, bold: true };
        s.save_to(&p).unwrap();
        assert_eq!(Settings::load_from(&p).appearance, s.appearance);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn when_exists_setting_migration() {
        use crate::output_guard::WhenExists;
        let dir = std::env::temp_dir().join(format!("ffenc_we_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.json");
        // 0.3.2 file: defaults (Ask, temp files cleaned)
        std::fs::write(&p, r#"{"crf": 20}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!((s.crf, s.when_exists, s.keep_temp_files), (20, WhenExists::Ask, false));
        std::fs::write(&p, r#"{"crf": 21, "when_exists": "Sometimes"}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!((s.crf, s.when_exists), (21, WhenExists::Ask));
        for w in WhenExists::ALL {
            let mut s = Settings::default();
            s.when_exists = w;
            s.keep_temp_files = true;
            s.save_to(&p).unwrap();
            let l = Settings::load_from(&p);
            assert_eq!((l.when_exists, l.keep_temp_files), (w, true));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scale_steps() {
        assert_eq!(clamp_scale(f32::NAN, SCALE_MIN, SCALE_MAX), 1.0);
        assert_eq!(clamp_scale(0.1, SCALE_MIN, SCALE_MAX), 0.75);
        assert_eq!(clamp_scale(1.234, SCALE_MIN, SCALE_MAX), 1.23);
        assert_eq!(step_scale(1.0, true), 1.1);
        assert_eq!(step_scale(1.0, false), 0.9);
        assert_eq!(step_scale(1.3, true), 1.5);
        assert_eq!(step_scale(1.3, false), 1.25);
        assert_eq!(step_scale(3.0, true), 3.0);
        assert_eq!(step_scale(0.75, false), 0.75);
        assert_eq!(step_scale(7.0, false), 2.5);
    }

    #[test]
    fn update_checks_enabled_by_default_for_new_users() {
        let s = Settings::default();
        assert!(!s.never_check_updates, "'Turn off Update Check for FFMPEG' is unchecked by default");
        assert!(s.auto_check_updates);
        assert!(s.may_check_updates(CheckTrigger::Automatic, 10 * AUTO_CHECK_INTERVAL_S, FfmpegInUse::AppManaged));
        // no settings file at all = first run
        let missing = std::env::temp_dir().join(format!("ffenc_nofile_{}", crate::util::unique_id())).join("s.json");
        assert!(!Settings::load_from(&missing).never_check_updates);
    }

    #[test]
    fn old_settings_keep_their_update_behaviour() {
        // 0.5.0 and older: automatic checks on (explicit or the old default) -> stay on
        let on = Settings::from_json(r#"{"crf": 20, "auto_check_updates": true, "last_update_check": 5}"#).unwrap();
        assert!(!on.never_check_updates && on.auto_check_updates);
        let implicit = Settings::from_json(r#"{"crf": 20}"#).unwrap();
        assert!(!implicit.never_check_updates, "old default was 'check at startup'");
        // they had turned automatic checks off -> never check
        let off = Settings::from_json(r#"{"auto_check_updates": false}"#).unwrap();
        assert!(off.never_check_updates && !off.auto_check_updates);
        // 0.5.1 files: the new key wins
        let new = Settings::from_json(r#"{"auto_check_updates": true, "never_check_updates": true}"#).unwrap();
        assert!(new.never_check_updates && !new.auto_check_updates);
        // saved files carry both keys, consistent, and round-trip
        let dir = std::env::temp_dir().join(format!("ffenc_upd_{}", crate::util::unique_id()));
        let p = dir.join("s.json");
        let mut s = on.clone();
        s.never_check_updates = true;
        s.auto_check_updates = true; // stale mirror is fixed on save
        s.save_to(&p).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!((v["never_check_updates"].as_bool(), v["auto_check_updates"].as_bool()), (Some(true), Some(false)));
        assert!(Settings::load_from(&p).never_check_updates);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn update_check_scheduler() {
        use FfmpegInUse::*;
        let day = AUTO_CHECK_INTERVAL_S;
        let mut s = Settings { never_check_updates: false, last_update_check: 1000, ..Settings::default() };
        assert!(s.may_check_updates(CheckTrigger::Automatic, 1000 + day + 1, AppManaged));
        assert!(!s.may_check_updates(CheckTrigger::Automatic, 1000 + day - 1, AppManaged), "at most once a day");
        assert!(!s.may_check_updates(CheckTrigger::Automatic, 1000 + day + 1, NotFound), "not without FFmpeg");
        assert!(!s.may_check_updates(CheckTrigger::Automatic, u64::MAX, OwnFolder), "never for the user's own FFmpeg folder");
        s.never_check_updates = true;
        assert!(!s.may_check_updates(CheckTrigger::Automatic, u64::MAX, AppManaged), "never when the option is checked");
        // manual checks always allowed, whatever the setting or FFmpeg
        for never in [true, false] {
            s.never_check_updates = never;
            for f in [NotFound, AppManaged, OwnFolder] {
                assert!(s.may_check_updates(CheckTrigger::Manual, 0, f));
            }
        }
        // the option is greyed out only for the user's own folder
        assert!(Settings::update_check_option_applies(AppManaged));
        assert!(Settings::update_check_option_applies(NotFound));
        assert!(!Settings::update_check_option_applies(OwnFolder));
    }
}
