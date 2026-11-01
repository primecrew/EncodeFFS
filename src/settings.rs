//! Persisted settings (JSON next to the exe for portability, falling back to
//! %APPDATA%\FFmpegEncoder when the exe folder is not writable).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cmd::common::CompressorParams;
use crate::ffmpeg::locate::exe_dir;
use crate::updater::{UpdateChannel, UpdateSource};

pub const PORTABLE_FILE: &str = "FFmpegEncoder.settings.json";

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
    pub auto_check_updates: bool,
    /// Unix seconds of the last automatic update check.
    pub last_update_check: u64,
    /// Keep ffmpeg_error*.log even when the job succeeds.
    pub keep_logs_on_success: bool,
    /// Last-used values per mode (opaque JSON owned by each mode).
    pub modes: BTreeMap<String, serde_json::Value>,
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
            last_update_check: 0,
            keep_logs_on_success: false,
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

impl Settings {
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn load() -> Settings {
        Self::load_from(&settings_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
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
}
