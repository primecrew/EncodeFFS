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
pub mod output_guard;
pub mod presets;
pub mod settings;
pub mod updater;
pub mod util;

/// Application version (from Cargo.toml).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_NAME: &str = "FFmpeg Encoder";
