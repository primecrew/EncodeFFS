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

// ---- Edition identity (ui-experimental branch). The experimental build is a separate app:
// its own exe name, data folders, portable file names and installer id, so it never reads or
// writes the stable version's settings, presets or downloaded FFmpeg (except "Import settings
// from stable version", which only reads/copies them).
/// Shown after the version in the window title and About.
pub const EDITION: &str = "Experimental";
/// Folder name under %APPDATA% / %LOCALAPPDATA% (stable: "FFmpegEncoder").
pub const DATA_DIR_NAME: &str = "FFmpegEncoder-Experimental";
/// Stable version's folder name (read only, for importing).
pub const STABLE_DATA_DIR_NAME: &str = "FFmpegEncoder";
/// Prefix of the portable files next to the exe (stable: "FFmpegEncoder").
pub const FILE_STEM: &str = "FFmpegEncoder-Experimental";
pub const STABLE_FILE_STEM: &str = "FFmpegEncoder";

pub mod import_stable;
pub mod layout;
pub mod splash;

/// "FFmpeg Encoder 0.6.0-dev.2 (Experimental)"
pub fn full_title() -> String {
    if EDITION.is_empty() {
        format!("{APP_NAME} {APP_VERSION}")
    } else {
        format!("{APP_NAME} {APP_VERSION} ({EDITION})")
    }
}
