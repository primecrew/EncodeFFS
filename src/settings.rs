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
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { theme: ThemeChoice::System, scale: 1.0, mono_scale: 1.0 }
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

/// Thumbnail size in the Create Slideshow image list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlideshowThumbSize {
    Small,
    Medium,
    Large,
}

impl SlideshowThumbSize {
    pub const ALL: [SlideshowThumbSize; 3] = [SlideshowThumbSize::Small, SlideshowThumbSize::Medium, SlideshowThumbSize::Large];

    pub fn label(self) -> &'static str {
        match self {
            SlideshowThumbSize::Small => "Small",
            SlideshowThumbSize::Medium => "Medium",
            SlideshowThumbSize::Large => "Large",
        }
    }

    /// Display width of the thumbnail box, in points (height is 3/4 of this).
    pub fn pixels(self) -> f32 {
        match self {
            SlideshowThumbSize::Small => 96.0,
            SlideshowThumbSize::Medium => 160.0,
            SlideshowThumbSize::Large => 256.0,
        }
    }

    /// Decode size (a little larger than the display size for sharpness).
    pub fn decode_side(self) -> u32 {
        match self {
            SlideshowThumbSize::Small => 144,
            SlideshowThumbSize::Medium => 240,
            SlideshowThumbSize::Large => 384,
        }
    }
}

impl Default for SlideshowThumbSize {
    fn default() -> Self {
        SlideshowThumbSize::Medium
    }
}

/// Create Slideshow list display (filenames, thumbnails, pop-out). Stored next to the
/// main settings file so toggling a view option does not dirty the preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlideshowUi {
    pub show_filenames: bool,
    pub show_thumbnails: bool,
    pub thumb_size: SlideshowThumbSize,
    pub list_popped_out: bool,
    pub popout_w: f32,
    pub popout_h: f32,
}

impl Default for SlideshowUi {
    fn default() -> Self {
        SlideshowUi {
            show_filenames: true,
            show_thumbnails: true,
            thumb_size: SlideshowThumbSize::Medium,
            list_popped_out: false,
            popout_w: 520.0,
            popout_h: 640.0,
        }
    }
}

impl SlideshowUi {
    pub fn sanitized(mut self) -> Self {
        self.popout_w = self.popout_w.clamp(360.0, 4000.0);
        self.popout_h = self.popout_h.clamp(240.0, 4000.0);
        if !self.popout_w.is_finite() {
            self.popout_w = 520.0;
        }
        if !self.popout_h.is_finite() {
            self.popout_h = 640.0;
        }
        self
    }

    pub fn load() -> Self {
        Self::load_from(&slideshow_ui_path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<SlideshowUi>(&t).ok())
            .unwrap_or_default()
            .sanitized()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&slideshow_ui_path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&self.clone().sanitized()).unwrap_or_else(|_| "{}".into()))
    }
}

/// `slideshow-ui.json` next to the main settings file.
pub fn slideshow_ui_path() -> PathBuf {
    match settings_path().parent() {
        Some(dir) => dir.join("slideshow-ui.json"),
        None => roaming_dir().join("slideshow-ui.json"),
    }
}


/// Environment override for tests (`FFENC_VIDEOS_DIR`).
pub const VIDEOS_DIR_ENV: &str = "FFENC_VIDEOS_DIR";

/// The user's Videos folder: Windows Known Folder `FOLDERID_Videos`, else
/// `%USERPROFILE%\Videos` or `~/Videos` (or `$HOME` if that does not exist).
pub fn default_videos_dir() -> PathBuf {
    if let Some(d) = std::env::var_os(VIDEOS_DIR_ENV) {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    if let Some(p) = known_folder_videos() {
        return p;
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    if let Some(h) = home {
        let videos = h.join("Videos");
        if videos.is_dir() || !h.join("Videos").exists() {
            // Prefer ~/Videos even if it does not exist yet (Windows first-run).
            return videos;
        }
        return h;
    }
    std::env::temp_dir()
}

pub fn default_videos_dir_string() -> String {
    default_videos_dir().to_string_lossy().into_owned()
}

fn known_folder_videos() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        return known_folder_videos_win();
    }
    #[cfg(not(windows))]
    {
        if let Some(d) = std::env::var_os("XDG_VIDEOS_DIR") {
            if !d.is_empty() {
                return Some(PathBuf::from(d));
            }
        }
        None
    }
}

/// `SHGetKnownFolderPath(FOLDERID_Videos)` — no extra windows-sys features required.
#[cfg(windows)]
fn known_folder_videos_win() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[repr(C)]
    struct Guid {
        d1: u32,
        d2: u16,
        d3: u16,
        d4: [u8; 8],
    }
    // FOLDERID_Videos = {18989B1D-99B5-455B-841C-AB7C74E4DDFC}
    const FOLDERID_VIDEOS: Guid = Guid { d1: 0x1898_9B1D, d2: 0x99B5, d3: 0x455B, d4: [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC] };
    extern "system" {
        fn SHGetKnownFolderPath(rfid: *const Guid, dw_flags: u32, h_token: isize, path: *mut *mut u16) -> i32;
        fn CoTaskMemFree(pv: *mut core::ffi::c_void);
    }
    unsafe {
        let mut p: *mut u16 = std::ptr::null_mut();
        if SHGetKnownFolderPath(&FOLDERID_VIDEOS, 0, 0, &mut p) != 0 || p.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *p.add(len) != 0 {
            len += 1;
        }
        let s = std::ffi::OsString::from_wide(std::slice::from_raw_parts(p, len));
        CoTaskMemFree(p.cast());
        Some(PathBuf::from(s))
    }
}

fn nonempty_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
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
    /// Theme, text size (0.3.2; older files get the defaults).
    #[serde(deserialize_with = "lenient")]
    pub appearance: Appearance,
    /// What to do when an output file already exists (0.3.3).
    #[serde(deserialize_with = "lenient_default")]
    pub when_exists: crate::output_guard::WhenExists,
    /// Keep chapter/list files and partial outputs and log their paths (debugging, 0.3.3).
    pub keep_temp_files: bool,
    /// Audio & subtitles: added tracks may differ from the video by this many seconds before a warning (0.4).
    pub duration_tolerance_s: f64,
    /// Show the "Encoding Finished" popup when jobs succeed (on by default).
    pub show_finished_popup: bool,
    /// Remember window layout (0.5.2; on by default): window place/size, panel sizes, dialog sizes and
    /// open sections are saved in their own file next to this one and restored at the next start.
    pub remember_layout: bool,
    /// App-managed FFmpeg location (0.5.2): the folder for the copy the app downloads/updates
    /// (`<folder>\bin`). Empty = the default `%LOCALAPPDATA%\FFmpegEncoder\ffmpeg`.
    pub managed_ffmpeg_dir: String,
    /// Legacy (0.1.x): last-used values per mode; migrated into presets on first start of 0.2.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ffmpeg_bin_path: String::new(),
            input_folder: default_videos_dir_string(),
            output_folder: default_videos_dir_string(),
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
            show_finished_popup: true,
            remember_layout: true,
            managed_ffmpeg_dir: String::new(),
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

/// `%APPDATA%\<name>` (Linux: ~/.config/<name>).
pub fn roaming_dir_named(name: &str) -> PathBuf {
    if let Some(d) = std::env::var_os("FFENC_ROAMING_BASE") {
        return PathBuf::from(d).join(name);
    }
    if let Some(d) = std::env::var_os("APPDATA") {
        return PathBuf::from(d).join(name);
    }
    if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(d).join(name);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h).join(".config").join(name);
    }
    std::env::temp_dir().join(name)
}

/// This edition's %APPDATA% folder (`FFmpegEncoder`).
pub fn roaming_dir() -> PathBuf {
    roaming_dir_named(crate::DATA_DIR_NAME)
}

/// Marker file the installer puts next to the exe (0.5.3): an installed copy keeps its settings in
/// %APPDATA%, never next to the exe.
pub const INSTALLED_MARKER: &str = "FFmpegEncoder.installed";

/// Where settings live: next to the exe if that folder is writable (portable),
/// otherwise %APPDATA%\FFmpegEncoder\settings.json. Installed copies (marker file next to the
/// exe) always use %APPDATA%.
pub fn settings_path() -> PathBuf {
    settings_path_in(&exe_dir(), &roaming_dir())
}

pub fn settings_path_in(exe_dir: &Path, roaming: &Path) -> PathBuf {
    if exe_dir.join(INSTALLED_MARKER).exists() {
        return roaming.join("settings.json");
    }
    let portable = exe_dir.join(PORTABLE_FILE);
    if (portable.exists() && std::fs::OpenOptions::new().append(true).open(&portable).is_ok()) || dir_writable(exe_dir) {
        return portable;
    }
    roaming.join("settings.json")
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


    /// If input/output folders are unset (empty), seed them once from the user's last
    /// draft or saved preset so upgrading does not drop a remembered folder; otherwise
    /// use the Videos folder. Existing non-empty settings are left alone.
    pub fn migrate_app_folders(&mut self, presets: &[crate::presets::Preset]) {
        let need_out = self.output_folder.trim().is_empty();
        let need_in = self.input_folder.trim().is_empty();
        if !need_out && !need_in {
            return;
        }
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        let mut consider = |v: &serde_json::Value| {
            if let Some(s) = nonempty_field(v, "out_dir") {
                outputs.push(s);
            }
            if let Some(s) = nonempty_field(v, "in_dir").or_else(|| nonempty_field(v, "src_dir")) {
                inputs.push(s);
            }
        };
        if let Some(d) = self.drafts.get(&self.last_preset) {
            consider(d);
        }
        if let Some(p) = presets.iter().find(|p| p.id == self.last_preset) {
            consider(&p.values);
        }
        for (id, d) in &self.drafts {
            if id != &self.last_preset {
                consider(d);
            }
        }
        for p in presets {
            if p.id != self.last_preset {
                consider(&p.values);
            }
        }
        if need_out {
            self.output_folder = outputs.into_iter().next().unwrap_or_else(default_videos_dir_string);
        }
        if need_in {
            self.input_folder = inputs.into_iter().next().unwrap_or_else(default_videos_dir_string);
        }
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
    fn defaults_are_generic_first_run() {
        let s = Settings::default();
        assert!(s.ffmpeg_bin_path.is_empty(), "FFmpeg path is detected, not hardcoded");
        assert_eq!(s.input_folder, default_videos_dir_string());
        assert_eq!(s.output_folder, default_videos_dir_string());
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
        assert_eq!(s.appearance, Appearance { theme: ThemeChoice::Light, scale: 3.0, mono_scale: 0.8 });
        std::fs::write(&p, r#"{"crf": 22, "appearance": {"theme": "Neon", "scale": "big"}}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!((s.crf, s.appearance.clone()), (22, Appearance::default()));
        std::fs::write(&p, r#"{"crf": 23, "appearance": "dark"}"#).unwrap();
        assert_eq!(Settings::load_from(&p).crf, 23);
        // round trip
        let mut s = Settings::default();
        s.appearance = Appearance { theme: ThemeChoice::HighContrast, scale: 2.5, mono_scale: 1.25 };
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

    #[test]
    fn remember_layout_default_on_and_saved() {
        assert!(Settings::default().remember_layout);
        assert!(Settings::from_json(r#"{"crf": 20}"#).unwrap().remember_layout, "older files: on");
        let off = Settings::from_json(r#"{"remember_layout": false}"#).unwrap();
        assert!(!off.remember_layout);
        let dir = std::env::temp_dir().join(format!("ffenc_rl_{}", crate::util::unique_id()));
        let p = dir.join("s.json");
        off.save_to(&p).unwrap();
        assert!(!Settings::load_from(&p).remember_layout);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn managed_ffmpeg_dir_setting() {
        // existing settings files (no field) keep the default location
        let old = Settings::from_json(r#"{"crf": 20}"#).unwrap();
        assert_eq!(old.managed_ffmpeg_dir, "");
        assert_eq!(Settings::default().managed_ffmpeg_dir, "");
        // a chosen folder (spaces, unicode, UNC) survives save/load unchanged
        let dir = std::env::temp_dir().join(format!("ffenc_mfd_{}", crate::util::unique_id()));
        let p = dir.join("s.json");
        for v in [r"E:\Video Tools\FFmpeg ö", r"\\nas\share\ffmpeg"] {
            let mut s = Settings::default();
            s.managed_ffmpeg_dir = v.into();
            s.save_to(&p).unwrap();
            assert_eq!(Settings::load_from(&p).managed_ffmpeg_dir, v);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finished_popup_setting_and_old_bold_field() {
        assert!(Settings::default().show_finished_popup);
        // older files: no key -> on; a leftover "bold" (option removed) is ignored and not written back
        let s = Settings::from_json(r#"{"crf": 20, "appearance": {"theme": "Dark", "bold": true}}"#).unwrap();
        assert!(s.show_finished_popup);
        assert_eq!(s.appearance.theme, ThemeChoice::Dark);
        let off = Settings::from_json(r#"{"show_finished_popup": false}"#).unwrap();
        assert!(!off.show_finished_popup);
        let dir = std::env::temp_dir().join(format!("ffenc_fp_{}", crate::util::unique_id()));
        let p = dir.join("s.json");
        off.save_to(&p).unwrap();
        assert!(!Settings::load_from(&p).show_finished_popup);
        s.save_to(&p).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("bold"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn portable_vs_installed_settings_location() {
        let root = std::env::temp_dir().join(format!("ffenc_inst_{}", crate::util::unique_id()));
        let (exe, roam) = (root.join("app"), root.join("roaming").join(crate::DATA_DIR_NAME));
        std::fs::create_dir_all(&exe).unwrap();
        assert_eq!(settings_path_in(&exe, &roam), exe.join(PORTABLE_FILE), "portable: next to the exe");
        std::fs::write(exe.join(INSTALLED_MARKER), "installed").unwrap();
        assert_eq!(settings_path_in(&exe, &roam), roam.join("settings.json"), "installed: %APPDATA%");
        assert!(INSTALLED_MARKER.starts_with(crate::FILE_STEM));
        assert_ne!(INSTALLED_MARKER, "FFmpegEncoder-Experimental.installed");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn slideshow_ui_defaults_and_roundtrip() {
        assert_eq!(SlideshowUi::default().show_filenames, true);
        assert_eq!(SlideshowUi::default().thumb_size, SlideshowThumbSize::Medium);
        let dir = std::env::temp_dir().join(format!("ffenc_ssui_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("slideshow-ui.json");
        let u = SlideshowUi { show_filenames: false, list_popped_out: true, popout_w: 800.0, popout_h: 900.0, ..Default::default() };
        u.save_to(&p).unwrap();
        let got = SlideshowUi::load_from(&p);
        assert_eq!(got.show_filenames, false);
        assert!(got.list_popped_out);
        assert_eq!(got.popout_w, 800.0);
        let bad = SlideshowUi { popout_w: f32::NAN, popout_h: 20.0, ..Default::default() }.sanitized();
        assert_eq!(bad.popout_w, 520.0);
        assert_eq!(bad.popout_h, 240.0);
        std::fs::write(&p, r#"{"show_filenames": 1}"#).unwrap();
        let fallback = SlideshowUi::load_from(&p);
        assert_eq!(fallback, SlideshowUi::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn migrate_app_folders_seeds_from_last_preset_and_leaves_existing() {
        let vids = default_videos_dir_string();
        // already set: unchanged
        let mut s = Settings { output_folder: r"D:\Videos\Out".into(), input_folder: r"D:\Videos\Inbox".into(), ffmpeg_bin_path: r"C:\ffmpeg\bin".into(), ..Settings::default() };
        s.migrate_app_folders(&[]);
        assert_eq!(s.output_folder, r"D:\Videos\Out");
        assert_eq!(s.input_folder, r"D:\Videos\Inbox");
        assert_eq!(s.ffmpeg_bin_path, r"C:\ffmpeg\bin");

        // empty: Videos when nothing to seed from
        let mut s = Settings { output_folder: String::new(), input_folder: String::new(), ..Settings::default() };
        s.migrate_app_folders(&[]);
        assert_eq!(s.output_folder, vids);
        assert_eq!(s.input_folder, vids);

        // empty: seed from last preset draft first, then saved preset values
        let mut s = Settings {
            output_folder: String::new(),
            input_folder: String::new(),
            last_preset: "builtin-reencode".into(),
            drafts: [("builtin-reencode".into(), serde_json::json!({"out_dir": "/draft/out", "in_dir": "/draft/in"}))].into(),
            ..Settings::default()
        };
        let presets = vec![crate::presets::Preset {
            id: "builtin-reencode".into(),
            values: serde_json::json!({"out_dir": "/preset/out", "src_dir": "/preset/src"}),
            ..crate::presets::factory("builtin-reencode").unwrap()
        }];
        s.migrate_app_folders(&presets);
        assert_eq!(s.output_folder, "/draft/out");
        assert_eq!(s.input_folder, "/draft/in");

        let mut s = Settings {
            output_folder: String::new(),
            input_folder: String::new(),
            last_preset: "builtin-reencode".into(),
            ..Settings::default()
        };
        s.migrate_app_folders(&presets);
        assert_eq!(s.output_folder, "/preset/out");
        assert_eq!(s.input_folder, "/preset/src");
    }

    #[test]
    fn videos_dir_env_override() {
        let dir = std::env::temp_dir().join(format!("ffenc_vid_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        // safety: this test is single-threaded in this module's default harness for this fn
        // we only check that a non-empty override is honoured when set at call time
        let old = std::env::var_os(VIDEOS_DIR_ENV);
        std::env::set_var(VIDEOS_DIR_ENV, &dir);
        assert_eq!(default_videos_dir(), dir);
        match old {
            Some(v) => std::env::set_var(VIDEOS_DIR_ENV, v),
            None => std::env::remove_var(VIDEOS_DIR_ENV),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
