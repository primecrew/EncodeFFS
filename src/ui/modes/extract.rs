//! Mode 4 – extract audio to PCM WAV.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::extract::{self, ExtractParams};
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
}

#[derive(Default)]
pub struct ExtractMode {
    st: State,
    probe: ProbeSlot,
}

fn set_input(st: &mut State, p: &std::path::Path) {
    st.input = s(p);
    auto_name(&mut st.out_name, &mut st.last_auto, format!("{}_audio.wav", stem(p)));
}

impl Mode for ExtractMode {
    fn key(&self) -> &'static str {
        "extract"
    }
    fn title(&self) -> &'static str {
        "4. Extract audio to WAV"
    }
    fn description(&self) -> &'static str {
        "PCM 16-bit stereo WAV at the source sample rate (fallback from Settings)."
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        self.probe.update(cx.egui, cx.ffprobe, &st.input);
        egui::Grid::new("m4").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Input video", &mut st.input, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder) {
                let p = PathBuf::from(st.input.trim());
                set_input(st, &p);
            }
            ui.label("");
            self.probe.status_ui(ui, describe_audio);
            ui.end_row();
            output_rows(ui, &mut st.out_dir, &mut st.out_name, &cx.settings.output_folder);
        });
        if let Some(i) = self.probe.info() {
            if i.audio_count() > 0 && i.first_audio_sample_rate().is_none() {
                warn(ui, format!("Could not determine audio sample rate. Using default: {} Hz", cx.settings.fallback_sample_rate));
            }
        }
    }
    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let input = check_input(&st.input, VIDEO_EXTS, "input video")?;
        let info = self.probe.require("Input video")?;
        if info.audio_count() == 0 {
            return Err(format!("No audio stream found in the input video: {}", input.display()));
        }
        let mut name = st.out_name.trim().to_string();
        if ffenc::util::ext_lower(std::path::Path::new(&name)) != "wav" {
            name = format!("{}.wav", stem(std::path::Path::new(&name)));
        }
        let output = check_output(&st.out_dir, &name)?;
        let sr = info.first_audio_sample_rate().unwrap_or(cx.settings.fallback_sample_rate);
        let cmd = extract::build(&ExtractParams { input: input.clone(), output: output.clone(), sample_rate: sr })?;
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Extract WAV -> {name}"), cmd, output.clone(), log_path_for(&output), info.any_duration())],
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
        self.st = restore(v, State { out_dir: s.output_folder.clone(), ..Default::default() });
        self.probe = ProbeSlot::default();
    }
}
