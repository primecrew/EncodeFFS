//! Mode 3 – add a commentary track.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::commentary::{self, CommentaryParams};
use ffenc::cmd::reencode::is_mp4_like;
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{stem, AUDIO_EXTS, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    video: String,
    commentary: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    #[serde(default = "sfx")]
    suffix: String,
    title: String,
    bitrate_k: u32,
    faststart: bool,
}
impl State {
    fn from_settings(s: &Settings) -> Self {
        State {
            video: String::new(),
            commentary: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: sfx(),
            title: s.commentary_name.clone(),
            bitrate_k: s.audio_bitrate_k,
            faststart: false,
        }
    }
}
impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

#[derive(Default)]
pub struct CommentaryMode {
    st: State,
    vprobe: ProbeSlot,
    aprobe: ProbeSlot,
}

fn sfx() -> String {
    "_with_commentary".into()
}

fn set_video(st: &mut State, p: &std::path::Path) {
    st.video = s(p);
    auto_name(&mut st.out_name, &mut st.last_auto, format!("{}{}.mp4", stem(p), st.suffix));
}

impl Mode for CommentaryMode {
    fn key(&self) -> &'static str {
        "commentary"
    }
    fn title(&self) -> &'static str {
        "Add commentary track"
    }
    fn description(&self) -> &'static str {
        "Copy video + original audio and add an AAC commentary track with a title."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["video", "commentary", "out_name", "last_auto"]
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        self.vprobe.update(cx.egui, cx.ffprobe, &st.video);
        self.aprobe.update(cx.egui, cx.ffprobe, &st.commentary);
        egui::Grid::new("m3").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Video", &mut st.video, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder) {
                let p = PathBuf::from(st.video.trim());
                set_video(st, &p);
            }
            ui.label("");
            self.vprobe.status_ui(ui, describe_video);
            ui.end_row();
            path_row(ui, "Commentary audio", &mut st.commentary, Pick::File("Audio", AUDIO_EXTS), &cx.settings.input_folder);
            ui.label("");
            self.aprobe.status_ui(ui, describe_audio);
            ui.end_row();
            if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) && !st.video.trim().is_empty() {
                let p = PathBuf::from(st.video.trim());
                set_video(st, &p);
            }
            ui.label("Track name");
            ui.text_edit_singleline(&mut st.title);
            ui.end_row();
            bitrate_ui(ui, "Commentary bitrate", &mut st.bitrate_k);
            faststart_ui(ui, &mut st.faststart, is_mp4_like(&PathBuf::from(&st.out_name)));
        });
    }
    fn build(&mut self, _cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let video = check_input(&st.video, VIDEO_EXTS, "input video")?;
        let commentary = check_input(&st.commentary, AUDIO_EXTS, "commentary audio")?;
        let info = self.vprobe.require("Video")?;
        let output = check_output(&st.out_dir, &st.out_name)?;
        if same_file(&video, &output) {
            return Err("Output must be different from the input.".into());
        }
        let pr = CommentaryParams {
            video,
            commentary,
            output: output.clone(),
            title: st.title.clone(),
            bitrate_k: st.bitrate_k,
            faststart: st.faststart,
            original_audio_count: info.audio_count(),
        };
        let cmd = commentary::build(&pr)?;
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Add commentary -> {}", st.out_name), cmd, output.clone(), log_path_for(&output), info.duration())],
            ..Default::default()
        })
    }
    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(p) = first_with_ext(paths, VIDEO_EXTS) {
            set_video(&mut self.st, p);
        }
        if let Some(p) = first_with_ext(paths, AUDIO_EXTS) {
            self.st.commentary = s(p);
        }
    }
    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        self.vprobe = ProbeSlot::default();
        self.aprobe = ProbeSlot::default();
    }
}
