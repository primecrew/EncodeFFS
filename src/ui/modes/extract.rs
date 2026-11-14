//! Mode 4 – extract audio to PCM WAV.

use std::path::PathBuf;

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::extract::{self, ExtractParams};
use ffenc::ffmpeg::probe::MediaInfo;
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, stem, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum TrackChoice {
    #[default]
    Default,
    Specific,
    Each,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    input: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    #[serde(default = "sfx")]
    suffix: String,
    choice: TrackChoice,
    /// 0-based audio index used when `choice` is Specific.
    track: usize,
}

#[derive(Default)]
pub struct ExtractMode {
    st: State,
    probe: ProbeSlot,
}

fn sfx() -> String {
    "_audio".into()
}

fn set_input(st: &mut State, p: &std::path::Path) {
    st.input = s(p);
    auto_name(&mut st.out_name, &mut st.last_auto, format!("{}{}.wav", stem(p), st.suffix));
}

fn describe_extract(i: &MediaInfo, choice: TrackChoice, track: usize) -> String {
    let n = i.audio_count();
    if n == 0 {
        return "No audio stream found".into();
    }
    if choice == TrackChoice::Each && n > 1 {
        return format!(
            "{} · {} audio tracks, each to its own WAV at that track's sample rate",
            i.any_duration().map(fmt_hms).unwrap_or_else(|| "?".into()),
            n
        );
    }
    let idx = match choice {
        TrackChoice::Specific if n > 1 => track.min(n - 1),
        _ => i.default_audio_index().unwrap_or(0),
    };
    let a = match i.audio_at(idx) {
        Some(a) => a,
        None => return "No audio stream found".into(),
    };
    let mut line = format!(
        "{} · {} · {} Hz · {} ch",
        i.any_duration().map(fmt_hms).unwrap_or_else(|| "?".into()),
        a.codec_name,
        a.sample_rate.map(|s| s.to_string()).unwrap_or_else(|| "?".into()),
        a.channels.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
    );
    if n > 1 {
        line.push_str(" · ");
        line.push_str(&a.audio_label(idx + 1));
    }
    line
}

fn selected_indexes(info: &MediaInfo, choice: TrackChoice, track: usize) -> Vec<usize> {
    let n = info.audio_count();
    if n == 0 {
        return vec![];
    }
    match choice {
        TrackChoice::Each if n > 1 => (0..n).collect(),
        TrackChoice::Specific if n > 1 => vec![track.min(n - 1)],
        _ => vec![info.default_audio_index().unwrap_or(0)],
    }
}

impl Mode for ExtractMode {
    fn key(&self) -> &'static str {
        "extract"
    }
    fn title(&self) -> &'static str {
        "Extract audio to WAV"
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
            self.probe.status_ui(ui, |i| describe_extract(i, st.choice, st.track));
            ui.end_row();
            if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) && !st.input.trim().is_empty() {
                let p = PathBuf::from(st.input.trim());
                set_input(st, &p);
            }
        });
        if let Some(i) = self.probe.info() {
            let n = i.audio_count();
            if n > 1 {
                let def = i.default_audio_index().unwrap_or(0);
                let label = i.audio_at(def).map(|a| a.audio_label(def + 1)).unwrap_or_else(|| format!("Track {}", def + 1));
                warn(ui, format!("This file has {n} audio tracks. Only the default track ({label}) will be extracted unless you choose otherwise."));
                ui.horizontal_wrapped(|ui| {
                    ui.radio_value(&mut st.choice, TrackChoice::Default, "Default track").on_hover_text("Same as a single-track file: FFmpeg picks the default track. The command is unchanged.");
                    ui.radio_value(&mut st.choice, TrackChoice::Specific, "A specific track").on_hover_text("Extract only the track you pick.");
                    ui.radio_value(&mut st.choice, TrackChoice::Each, "Each track to its own WAV").on_hover_text("One WAV per audio track. Names include the track number and title.");
                });
                if st.choice == TrackChoice::Specific {
                    if st.track >= n {
                        st.track = def.min(n - 1);
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Track"));
                        let current = i.audio_at(st.track).map(|a| {
                            let mut s = a.audio_label(st.track + 1);
                            if st.track == def {
                                s.push_str(" (default)");
                            }
                            s
                        }).unwrap_or_else(|| format!("Track {}", st.track + 1));
                        egui::ComboBox::from_id_salt("extract_track").selected_text(current).show_ui(ui, |ui| {
                            for idx in 0..n {
                                let mut s = i.audio_at(idx).map(|a| a.audio_label(idx + 1)).unwrap_or_else(|| format!("Track {}", idx + 1));
                                if idx == def {
                                    s.push_str(" (default)");
                                }
                                ui.selectable_value(&mut st.track, idx, s);
                            }
                        });
                    });
                }
            }
            let missing_rate = selected_indexes(i, st.choice, st.track).iter().any(|&k| i.audio_at(k).and_then(|a| a.sample_rate).is_none());
            if n > 0 && missing_rate {
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
        let n = info.audio_count();
        let each = st.choice == TrackChoice::Each && n > 1;
        if each {
            let base = stem(std::path::Path::new(&name));
            let mut jobs = Vec::new();
            for (idx, a) in info.of_type("audio").enumerate() {
                let fname = extract::wav_name_for_track(&base, idx + 1, a.title.as_deref());
                let output = check_output(&st.out_dir, &fname)?;
                let sr = a.sample_rate.unwrap_or(cx.settings.fallback_sample_rate);
                let cmd = extract::build(&ExtractParams { input: input.clone(), output: output.clone(), sample_rate: sr, map_audio: Some(idx) })?;
                jobs.push(JobSpec::new(format!("Extract WAV -> {fname}"), cmd, output.clone(), log_path_for(&output), info.any_duration()));
            }
            return Ok(Plan { jobs, ..Default::default() });
        }
        let output = check_output(&st.out_dir, &name)?;
        let idx = match st.choice {
            TrackChoice::Specific if n > 1 => st.track.min(n - 1),
            _ => info.default_audio_index().unwrap_or(0),
        };
        let sr = info.audio_at(idx).and_then(|a| a.sample_rate).unwrap_or(cx.settings.fallback_sample_rate);
        // Default track (and a single-track file) omit -map so the command stays byte-identical.
        let map_audio = if st.choice == TrackChoice::Specific && n > 1 { Some(idx) } else { None };
        let cmd = extract::build(&ExtractParams { input: input.clone(), output: output.clone(), sample_rate: sr, map_audio })?;
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
        self.st = restore(v, State { out_dir: s.output_folder.clone(), suffix: sfx(), ..Default::default() });
        self.probe = ProbeSlot::default();
    }
}
