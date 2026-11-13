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
pub const APP_NAME: &str = "Encode FFS";
/// Visible publisher / developer name (About box, installer, version info).
pub const PUBLISHER: &str = "PrimeCrew Software";

/// Folder name under %APPDATA% / %LOCALAPPDATA%.
pub const DATA_DIR_NAME: &str = "FFmpegEncoder";
/// Prefix of the portable files next to the exe.
pub const FILE_STEM: &str = "FFmpegEncoder";
/// Other editions' data folders (their downloaded FFmpeg is never used or moved by this app).
pub const OTHER_EDITION_DATA_DIR_NAMES: &[&str] = &[
    "FFmpegEncoder-Experimental",
    "FFmpegEncoder-Codecs",
    "FFmpegEncoder-Slideshow", // archived standalone edition
];

/// Window title, e.g. "Encode FFS 0.6.0-rc.3".
pub fn full_title() -> String {
    format!("{APP_NAME} {APP_VERSION}")
}

pub mod job_history;
pub mod layout;
