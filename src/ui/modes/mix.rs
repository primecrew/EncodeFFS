//! Mode 6 – mix all audio tracks into one AAC track (MKV).

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::mix::{self, MixParams};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{stem, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    input: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    #[serde(default = "sfx")]
    suffix: String,
    bitrate_k: u32,
}

#[derive(Default)]
pub struct MixMode {
    st: State,
    probe: ProbeSlot,
}

fn sfx() -> String {
    "_mixed".into()
}

fn set_input(st: &mut State, p: &std::path::Path) {
    st.input = s(p);
    auto_name(&mut st.out_name, &mut st.last_auto, format!("{}{}.mkv", stem(p), st.suffix));
}

impl Mode for MixMode {
    fn key(&self) -> &'static str {
        "mix"
    }
    fn title(&self) -> &'static str {
        "Mix audio tracks to one (MKV)"
    }
    fn description(&self) -> &'static str {
        "amix all audio tracks (normalize=1) into one AAC track; video copied, chapters kept."
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        self.probe.update(cx.egui, cx.ffprobe, &st.input);
        egui::Grid::new("m6").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Input video", &mut st.input, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder) {
                let p = PathBuf::from(st.input.trim());
                set_input(st, &p);
            }
            ui.label("");
            self.probe.status_ui(ui, describe_video);
            ui.end_row();
            if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) && !st.input.trim().is_empty() {
                let p = PathBuf::from(st.input.trim());
                set_input(st, &p);
            }
            bitrate_ui(ui, "Mixed AAC bitrate", &mut st.bitrate_k);
        });
        if let Some(i) = self.probe.info() {
            if i.audio_count() < 2 {
                warn(ui, format!("Mixing requires at least two audio tracks. Found: {}", i.audio_count()));
            }
        }
    }
    fn build(&mut self, _cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let input = check_input(&st.input, VIDEO_EXTS, "input video")?;
        let info = self.probe.require("Input video")?;
        let output = check_output(&st.out_dir, &st.out_name)?;
        let pr = MixParams { input: input.clone(), output, audio_count: info.audio_count(), bitrate_k: st.bitrate_k };
        let cmd = mix::build(&pr)?;
        let real_out = PathBuf::from(cmd.args.last().cloned().unwrap_or_default());
        if same_file(&input, &real_out) {
            return Err("Output must be different from the input.".into());
        }
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Mix audio -> {}", real_out.file_name().unwrap_or_default().to_string_lossy()), cmd, real_out.clone(), log_path_for(&real_out), info.duration())],
            ..Default::default()
        })
    }
    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(p) = first_with_ext(paths, VIDEO_EXTS) {
            set_input(&mut self.st, p);
        }
    }
    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State { out_dir: s.output_folder.clone(), bitrate_k: s.audio_bitrate_k, suffix: sfx(), ..Default::default() });
        self.probe = ProbeSlot::default();
    }
}
