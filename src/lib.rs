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
pub mod recent_folders;
pub mod data_migrate;
pub mod updater;
pub mod util;
pub mod winver;

/// Application version (from Cargo.toml).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "Encode FFS";
/// Visible publisher / developer name (About box, installer, version info).
pub const PUBLISHER: &str = "PrimeCrew Software";
/// Short copyright line for the About box and the Windows version resource.
pub const COPYRIGHT: &str = "Copyright (C) 2026 PrimeCrew Software";

/// About-box line under the heading (1.1).
pub const ABOUT_TAGLINE: &str = "A dedicated front end for FFmpeg by Tolwyn";

/// About-box heading, e.g. "Encode FFS v1.0.0 by PrimeCrew Software, (C) 2026".
pub fn about_heading() -> String {
    format!("{APP_NAME} v{APP_VERSION} by {PUBLISHER}, (C) 2026")
}


/// Leaf folder under `%APPDATA%\<publisher>` / `%LOCALAPPDATA%\<publisher>`.
pub const DATA_DIR_NAME: &str = "Encode FFS";
/// Pre-0.6.0-rc.3 data folder name, directly under %APPDATA% / %LOCALAPPDATA%.
pub const LEGACY_DATA_DIR_NAME: &str = "FFmpegEncoder";
/// Prefix of the portable files next to the exe (settings, presets, layout, logs, installed marker).
/// Kept as FFmpegEncoder so a portable folder with an older exe name still finds its files.
pub const FILE_STEM: &str = "FFmpegEncoder";
/// File name of the Windows program (installer, portable zip, version resource).
pub const EXE_FILE: &str = "Encode FFS.exe";
/// Pre-rename program file, deleted on upgrade and still recognised by the uninstaller / “already running” check.
pub const LEGACY_EXE_FILE: &str = "FFmpegEncoder.exe";
/// Other editions' data folders, relative to %APPDATA% / %LOCALAPPDATA%
/// (their downloaded FFmpeg is never used or moved by this app).
pub const OTHER_EDITION_DATA_DIR_NAMES: &[&str] = &[
    "FFmpegEncoder-Experimental",
    "FFmpegEncoder-Codecs",
    "FFmpegEncoder-Slideshow", // archived standalone edition
    "PrimeCrew Software/Encode FFS (Codecs)",
];

/// Relative path of this edition's data folder (`PrimeCrew Software\Encode FFS`).
pub fn data_dir_rel() -> std::path::PathBuf {
    std::path::PathBuf::from(PUBLISHER).join(DATA_DIR_NAME)
}

/// Relative path of the pre-rebrand data folder (`FFmpegEncoder`).
pub fn legacy_data_dir_rel() -> std::path::PathBuf {
    std::path::PathBuf::from(LEGACY_DATA_DIR_NAME)
}

/// Window title, e.g. "Encode FFS 1.0.0".
pub fn full_title() -> String {
    format!("{APP_NAME} {APP_VERSION}")
}

pub mod job_history;
pub mod layout;
