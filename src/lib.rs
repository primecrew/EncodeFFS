//! Core (GUI-independent) logic for FFmpeg Encoder.
//!
//! * `cmd`      – pure functions that turn settings into ffmpeg argument vectors (one module per mode)
//! * `ffmpeg`   – locating ffmpeg/ffprobe, probing media, running jobs with progress
//! * `updater`  – discovering, downloading, verifying and installing FFmpeg builds
//! * `settings` – persisted JSON settings
//! * `presets`  – named, user-editable configurations (built-ins + custom)
//!
//! Everything here is unit-testable on Linux; the egui front end lives in the binary crate.

pub mod cmd;
pub mod cmdline;
pub mod ffmpeg;
pub mod finished;
pub mod output_guard;
pub mod presets;
pub mod settings;
pub mod updater;
pub mod util;

/// Application version (from Cargo.toml).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "FFmpeg Encoder";

// ---- Edition identity (slideshow branch). Separate app next to stable: own exe, data folders
// and portable file names. "Import settings from stable" only reads/copies them.
pub const EDITION: &str = "Slideshow";
pub const DATA_DIR_NAME: &str = "FFmpegEncoder-Slideshow";
pub const STABLE_DATA_DIR_NAME: &str = "FFmpegEncoder";
pub const FILE_STEM: &str = "FFmpegEncoder-Slideshow";
pub const STABLE_FILE_STEM: &str = "FFmpegEncoder";
pub const OTHER_EDITION_DATA_DIR_NAMES: &[&str] = &["FFmpegEncoder", "FFmpegEncoder-Experimental", "FFmpegEncoder-Codecs"];

pub mod import_stable;

/// "FFmpeg Encoder 0.5.4-slideshow.1 (Slideshow)"
pub fn full_title() -> String {
    format!("{APP_NAME} {APP_VERSION} ({EDITION})")
}

pub mod job_history;
pub mod layout;
