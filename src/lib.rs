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
pub mod data_migrate;
pub mod updater;
pub mod util;
pub mod winver;

/// Application version (from Cargo.toml).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "Encode FFS";
/// Visible publisher / developer name (About box, installer, version info).
pub const PUBLISHER: &str = "PrimeCrew Software";

// ---- Edition identity (codecs branch). A separate app next to the stable one: own exe name,
// data folders and portable file names, so it never writes the stable version's settings,
// presets or downloaded FFmpeg ("Import settings from stable" only reads/copies them).
/// Shown after the version in the window title.
pub const EDITION: &str = "Codecs";
/// Leaf folder under `%APPDATA%\<publisher>` / `%LOCALAPPDATA%\<publisher>`.
pub const DATA_DIR_NAME: &str = "Encode FFS (Codecs)";
/// Pre-rebrand data folder name, directly under %APPDATA% / %LOCALAPPDATA%.
pub const LEGACY_DATA_DIR_NAME: &str = "FFmpegEncoder-Codecs";
/// Stable edition's leaf folder (under the publisher folder).
pub const STABLE_DATA_DIR_NAME: &str = "Encode FFS";
/// Stable edition's pre-rebrand folder, directly under %APPDATA% / %LOCALAPPDATA%.
pub const LEGACY_STABLE_DATA_DIR_NAME: &str = "FFmpegEncoder";
/// Prefix of the portable files next to the exe (stable: "FFmpegEncoder").
pub const FILE_STEM: &str = "FFmpegEncoder-Codecs";
pub const STABLE_FILE_STEM: &str = "FFmpegEncoder";
/// Visible program file (official Windows artifacts). The Cargo bin name is still FILE_STEM.
pub const EXE_FILE: &str = "Encode FFS (Codecs).exe";
/// Previous program file, removed on installer upgrade.
pub const LEGACY_EXE_FILE: &str = "FFmpegEncoder-Codecs.exe";
/// Other editions' data folders, relative to %APPDATA% / %LOCALAPPDATA%
/// (their downloaded FFmpeg is never used or moved by this app).
pub const OTHER_EDITION_DATA_DIR_NAMES: &[&str] = &[
    "FFmpegEncoder",
    "FFmpegEncoder-Experimental",
    "FFmpegEncoder-Slideshow",
    "PrimeCrew Software/Encode FFS",
];

/// Relative path of this edition's data folder (`PrimeCrew Software\Encode FFS (Codecs)`).
pub fn data_dir_rel() -> std::path::PathBuf {
    std::path::PathBuf::from(PUBLISHER).join(DATA_DIR_NAME)
}

/// Relative path of this edition's pre-rebrand data folder (`FFmpegEncoder-Codecs`).
pub fn legacy_data_dir_rel() -> std::path::PathBuf {
    std::path::PathBuf::from(LEGACY_DATA_DIR_NAME)
}

/// Relative path of the current stable edition data folder.
pub fn stable_data_dir_rel() -> std::path::PathBuf {
    std::path::PathBuf::from(PUBLISHER).join(STABLE_DATA_DIR_NAME)
}

/// Relative path of the pre-rebrand stable data folder.
pub fn legacy_stable_data_dir_rel() -> std::path::PathBuf {
    std::path::PathBuf::from(LEGACY_STABLE_DATA_DIR_NAME)
}

pub mod import_stable;

/// Window title, e.g. "Encode FFS 0.5.5-codecs.3 (Codecs)".
pub fn full_title() -> String {
    format!("{APP_NAME} {APP_VERSION} ({EDITION})")
}

pub mod job_history;
pub mod layout;
