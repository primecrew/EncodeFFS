//! GUI modes. Each mode is a self-contained struct implementing [`Mode`].
//! To add a mode: write a pure builder in `ffenc::cmd`, add a module here
//! implementing `Mode`, and register it in [`all_modes`].

use std::path::{Path, PathBuf};

use eframe::egui;
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;

pub mod batch;
pub mod commentary;
pub mod custom;
pub mod extract;
pub mod frameserve;
pub mod join;
pub mod mix;
pub mod reencode;
pub mod remux;
pub mod replace_audio;
pub mod still;
pub mod subs;
pub mod tracks;

/// What a mode can see while drawing / building.
pub struct Ctx<'a> {
    pub settings: &'a Settings,
    pub ffprobe: Option<&'a Path>,
    pub egui: &'a egui::Context,
    /// Optional encoders of the current FFmpeg (None until it has been checked).
    pub encoders: Option<ffenc::cmd::codecs::Encoders>,
}

/// Jobs to preview and queue.
#[derive(Default)]
pub struct Plan {
    pub jobs: Vec<JobSpec>,
    /// (label, is_remux, pre-skipped count) for a batch summary.
    pub group: Option<(String, bool, usize)>,
    /// Extra lines for the preview (e.g. skipped files).
    pub notes: Vec<String>,
}

pub trait Mode {
    /// Stable key used for remembered values.
    fn key(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Whether libx264/aac must be present (all but remux).
    fn needs_encoders(&self) -> bool {
        true
    }
    /// Keys of `save()` that belong to one run (input files, typed output
    /// names) and are therefore not stored in a preset.
    fn run_fields(&self) -> &'static [&'static str] {
        &["input", "out_name", "last_auto"]
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx);
    /// Validate and build the jobs.
    fn build(&mut self, cx: &Ctx) -> Result<Plan, String>;
    /// Unusual setting combinations that warn but still run. Default: none.
    fn conflict_warnings(&self) -> Vec<ffenc::cmd::conflicts::Conflict> {
        vec![]
    }
    /// Apply one one-click fix from a warning. Default: no-op.
    fn apply_conflict_fix(&mut self, _kind: ffenc::cmd::conflicts::ConflictKind) {}
    /// Whether drops should keep folders as folders (folder-based presets).
    /// Otherwise the window expands each dropped folder into its files first.
    fn wants_folders(&self) -> bool {
        false
    }
    /// Files dropped onto the window while this mode is active.
    fn on_drop(&mut self, paths: &[PathBuf], cx: &Ctx);
    /// Remembered values.
    fn save(&self) -> serde_json::Value;
    /// Restore remembered values (`Null` = start from the settings defaults).
    fn load(&mut self, v: &serde_json::Value, s: &Settings);
}

/// Conflict-check fields shared by Re-encode, Batch, Join and Custom.
pub fn media_choices(
    video: &ffenc::cmd::common::VideoEncode,
    audio: &ffenc::cmd::common::AudioEncode,
    container: &str,
    out_name: &str,
    last_auto: &str,
    output: PathBuf,
    range_convert: bool,
    source_range: Option<ffenc::cmd::common::ColorRange>,
) -> ffenc::cmd::conflicts::Choices {
    ffenc::cmd::conflicts::Choices {
        video: video.clone(),
        audio: audio.clone(),
        container: container.to_string(),
        out_name: out_name.to_string(),
        last_auto: last_auto.to_string(),
        output,
        bits: ffenc::cmd::conflicts::script_bit_depth(video),
        bt709_tags: false,
        range_convert,
        source_range,
        uses_script_matrix: false,
        ..Default::default()
    }
}

pub fn apply_media_fix(
    kind: ffenc::cmd::conflicts::ConflictKind,
    video: &mut ffenc::cmd::common::VideoEncode,
    audio: &mut ffenc::cmd::common::AudioEncode,
    container: &mut String,
    out_name: &mut String,
    last_auto: &mut String,
    output: PathBuf,
    range_convert: Option<&mut bool>,
) {
    let current_range = range_convert.as_deref().copied().unwrap_or(false);
    let mut c = media_choices(video, audio, container, out_name, last_auto, output, current_range, None);
    ffenc::cmd::conflicts::apply(&mut c, kind);
    *video = c.video;
    *audio = c.audio;
    *container = c.container;
    *out_name = c.out_name;
    *last_auto = c.last_auto;
    if let Some(r) = range_convert {
        *r = c.range_convert;
    }
}

/// Deserialize remembered state or fall back to `default`.
pub fn restore<T: serde::de::DeserializeOwned>(v: &serde_json::Value, default: T) -> T {
    if v.is_null() {
        default
    } else {
        serde_json::from_value(v.clone()).unwrap_or(default)
    }
}

pub fn all_modes() -> Vec<Box<dyn Mode>> {
    vec![
        Box::new(reencode::ReencodeMode::default()),
        Box::new(tracks::TracksMode::default()),
        Box::new(still::StillMode::default()),
        Box::new(commentary::CommentaryMode::default()),
        Box::new(extract::ExtractMode::default()),
        Box::new(replace_audio::ReplaceAudioMode::default()),
        Box::new(mix::MixMode::default()),
        Box::new(batch::BatchMode::default()),
        Box::new(subs::SubsMode::default()),
        Box::new(remux::RemuxMode::default()),
        Box::new(join::JoinMode::default()),
        Box::new(frameserve::FrameserveMode::default()),
        Box::new(custom::CustomMode::default()),
    ]
}

/// A fresh, unloaded form for a preset type.
pub fn new_mode(kind: &str) -> Option<Box<dyn Mode>> {
    all_modes().into_iter().find(|m| m.key() == kind)
}

/// (kind, label) of every preset type.
pub fn kinds() -> Vec<(&'static str, &'static str)> {
    all_modes().iter().map(|m| (m.key(), m.title())).collect()
}

/// Defaults derived from settings, shared by several modes.
pub fn default_video(s: &Settings) -> ffenc::cmd::common::VideoEncode {
    ffenc::cmd::common::VideoEncode {
        crf: s.crf,
        preset: s.preset.clone(),
        tune: None,
        codec: s.video_codec,
        ten_bit: s.ten_bit,
        svt_preset: s.svt_preset,
        film_grain: s.film_grain,
        film_grain_denoise: s.film_grain_denoise,
        output_fps: String::new(),
    }
}

pub fn default_audio(s: &Settings) -> ffenc::cmd::common::AudioEncode {
    ffenc::cmd::common::AudioEncode {
        bitrate_k: s.audio_bitrate_k,
        gain_db: s.gain_db,
        compressor: ffenc::cmd::common::CompressorPreset::None,
        advanced: s.compressor,
        codec: s.audio_codec,
        ac3_bitrate_k: s.ac3_bitrate_k,
        source_channels: None,
    }
}
