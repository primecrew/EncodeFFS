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
pub const APP_NAME: &str = "FFS Just Encode";
/// Visible publisher / developer name (About box, installer, version info).
pub const PUBLISHER: &str = "PrimeCrew Software";

// ---- Edition identity (codecs branch). A separate app next to the stable one: own exe name,
// data folders and portable file names, so it never writes the stable version's settings,
// presets or downloaded FFmpeg ("Import settings from stable" only reads/copies them).
/// Shown after the version in the window title.
pub const EDITION: &str = "Codecs";
/// Folder name under %APPDATA% / %LOCALAPPDATA% (stable: "FFmpegEncoder").
pub const DATA_DIR_NAME: &str = "FFmpegEncoder-Codecs";
/// Stable version's folder name (read only, for importing).
pub const STABLE_DATA_DIR_NAME: &str = "FFmpegEncoder";
/// Prefix of the portable files next to the exe (stable: "FFmpegEncoder").
pub const FILE_STEM: &str = "FFmpegEncoder-Codecs";
pub const STABLE_FILE_STEM: &str = "FFmpegEncoder";
/// Other editions' folders (their downloaded FFmpeg is never used or moved by this app).
pub const OTHER_EDITION_DATA_DIR_NAMES: &[&str] = &["FFmpegEncoder", "FFmpegEncoder-Experimental"];

pub mod import_stable;

/// Window title, e.g. "FFS Just Encode 0.5.5-codecs.3 (Codecs)".
pub fn full_title() -> String {
    format!("{APP_NAME} {APP_VERSION} ({EDITION})")
}

pub mod job_history;
pub mod layout;
