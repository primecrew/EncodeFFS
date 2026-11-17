//! Mode 2 – MP4 from a still image + audio.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_audio, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::codecs::VideoCodec;
use ffenc::cmd::common::{AudioEncode, VideoEncode};
use ffenc::cmd::still::{self, still_default_video, StillParams};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, stem, AUDIO_EXTS, IMAGE_EXTS};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    image: String,
    audio: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    #[serde(default = "sfx")]
    suffix: String,
    use_audio_duration: bool,
    custom_duration: f64,
    video: VideoEncode,
    audio_encode: AudioEncode,
    faststart: bool,
    range_convert: bool,
}

impl State {
    fn from_settings(s: &Settings) -> Self {
        State {
            image: String::new(),
            audio: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: sfx(),
            use_audio_duration: true,
            custom_duration: 60.0,
            video: still_default_video(),
            audio_encode: AudioEncode { bitrate_k: s.audio_bitrate_k, ..default_audio(s) },
            faststart: false,
            range_convert: false,
        }
    }
}
impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

#[derive(Default)]
pub struct StillMode {
    st: State,
    probe: ProbeSlot,
}

fn sfx() -> String {
    "_still".into()
}

fn set_image(st: &mut State, p: &std::path::Path) {
    st.image = s(p);
    auto_name(&mut st.out_name, &mut st.last_auto, format!("{}{}.mp4", stem(p), st.suffix));
}

impl Mode for StillMode {
    fn key(&self) -> &'static str {
        "still"
    }
    fn title(&self) -> &'static str {
        "MP4 from still image"
    }
    fn description(&self) -> &'static str {
        "One image + one audio file -> MP4 video."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["image", "audio", "out_name", "last_auto"]
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        self.probe.update(cx.egui, cx.ffprobe, &st.audio);
        egui::Grid::new("m2").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Image", &mut st.image, Pick::File("Image", IMAGE_EXTS), &cx.settings.input_folder) {
                let p = PathBuf::from(st.image.trim());
                set_image(st, &p);
            }
            path_row(ui, "Audio", &mut st.audio, Pick::File("Audio", AUDIO_EXTS), &cx.settings.input_folder);
            ui.label("");
            self.probe.status_ui(ui, describe_audio);
            ui.end_row();
            crate::ui::common::row_label(ui, "Duration");
            ui.horizontal(|ui| {
                let ad = self.probe.info().and_then(|i| i.any_duration());
                ui.checkbox(&mut st.use_audio_duration, format!("Use audio duration{}", ad.map(|d| format!(" ({})", fmt_hms(d))).unwrap_or_default()));
                if !st.use_audio_duration {
                    num_f64(ui, &mut st.custom_duration, 0.01..=1.0e7, 0.1, " s");
                }
            });
            ui.end_row();
            if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) && !st.image.trim().is_empty() {
                let p = PathBuf::from(st.image.trim());
                set_image(st, &p);
            }
            let before = st.video.codec;
            crate::ui::codecs_ui::video_codec_ui(ui, "m2", &mut st.video, None, &cx.ff_status());
            if before != st.video.codec && st.video.codec == VideoCodec::H264 && st.video.tune.is_none() {
                st.video.tune = Some("stillimage".into());
            }
            crate::ui::codecs_ui::audio_codec_ui(ui, "m2", &mut st.audio_encode, false, &cx.ff_status());
            faststart_ui(ui, &mut st.faststart, true);
            range_ui(ui, &mut st.range_convert, None);
        });
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let image = check_input(&st.image, IMAGE_EXTS, "image file")?;
        let audio = check_input(&st.audio, AUDIO_EXTS, "audio file")?;
        let info = self.probe.require("Audio file")?;
        let audio_dur = info.any_duration().ok_or("Could not determine audio duration")?;
        let output = check_output(&st.out_dir, &st.out_name)?;
        let pr = StillParams {
            image,
            audio,
            output: output.clone(),
            video: st.video.clone(),
            audio_encode: st.audio_encode.clone(),
            duration_s: if st.use_audio_duration { None } else { Some(st.custom_duration) },
            faststart: st.faststart,
            range_convert: st.range_convert,
        };
        crate::ui::codecs_ui::check_available(cx, Some(&st.video), Some(&st.audio_encode))?;
        let cmd = still::build(&pr)?;
        let dur = pr.duration_s.unwrap_or(audio_dur);
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Still image -> {}", st.out_name), cmd, output.clone(), log_path_for(&output), Some(dur))],
            ..Default::default()
        })
    }

    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(p) = first_with_ext(paths, IMAGE_EXTS) {
            set_image(&mut self.st, p);
        }
        if let Some(p) = first_with_ext(paths, AUDIO_EXTS) {
            self.st.audio = s(p);
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        if v.get("audio_encode").is_none() {
            if let Some(k) = v.get("bitrate_k").and_then(|x| x.as_u64()) {
                self.st.audio_encode.bitrate_k = k as u32;
            }
        }
        if v.get("video").is_none() {
            self.st.video = still_default_video();
        }
        self.probe = ProbeSlot::default();
    }

    fn conflict_warnings(&self) -> Vec<ffenc::cmd::conflicts::Conflict> {
        let out = std::path::Path::new(&self.st.out_dir).join(&self.st.out_name);
        ffenc::cmd::conflicts::detect(&super::media_choices(
            &self.st.video,
            &self.st.audio_encode,
            "mp4",
            &self.st.out_name,
            &self.st.last_auto,
            out,
            self.st.range_convert,
            None,
        ))
    }

    fn apply_conflict_fix(&mut self, kind: ffenc::cmd::conflicts::ConflictKind) {
        let out = std::path::Path::new(&self.st.out_dir).join(&self.st.out_name);
        super::apply_media_fix(
            kind,
            &mut self.st.video,
            &mut self.st.audio_encode,
            &mut String::from("mp4"),
            &mut self.st.out_name,
            &mut self.st.last_auto,
            out,
            Some(&mut self.st.range_convert),
        );
    }
}
