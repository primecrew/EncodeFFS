//! Mode 5 – replace the primary audio with a WAV.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_audio, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::common::AudioEncode;
use ffenc::cmd::replace_audio::{self, ReplaceAudioParams};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{stem, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    video: String,
    wav: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    audio: AudioEncode,
}

#[derive(Default)]
pub struct ReplaceAudioMode {
    st: State,
    vprobe: ProbeSlot,
    wprobe: ProbeSlot,
}

fn set_video(st: &mut State, p: &std::path::Path) {
    st.video = s(p);
    auto_name(&mut st.out_name, &mut st.last_auto, format!("{}_newaudio.mp4", stem(p)));
}

/// Script's WAV validation: needs an audio stream; non-PCM only warns.
fn wav_check(i: &ffenc::ffmpeg::probe::MediaInfo) -> Result<Option<String>, String> {
    let a = i.of_type("audio").next().ok_or("No valid audio stream found in the WAV file")?;
    Ok((!a.codec_name.starts_with("pcm_")).then(|| format!("WAV file codec is {}. Expected PCM format, but proceeding as FFmpeg may handle it.", a.codec_name)))
}

impl Mode for ReplaceAudioMode {
    fn key(&self) -> &'static str {
        "replace_audio"
    }
    fn title(&self) -> &'static str {
        "5. Replace primary audio with WAV"
    }
    fn description(&self) -> &'static str {
        "Copy the video, replace all audio with the WAV encoded to AAC (gain/compressor optional)."
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        self.vprobe.update(cx.egui, cx.ffprobe, &st.video);
        self.wprobe.update(cx.egui, cx.ffprobe, &st.wav);
        egui::Grid::new("m5").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Video", &mut st.video, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder) {
                let p = PathBuf::from(st.video.trim());
                set_video(st, &p);
            }
            ui.label("");
            self.vprobe.status_ui(ui, describe_video);
            ui.end_row();
            path_row(ui, "Replacement WAV", &mut st.wav, Pick::File("WAV", &["wav"]), &cx.settings.input_folder);
            ui.label("");
            self.wprobe.status_ui(ui, describe_audio);
            ui.end_row();
            output_rows(ui, &mut st.out_dir, &mut st.out_name, &cx.settings.output_folder);
            audio_encode_ui(ui, "m5a", &mut st.audio);
        });
        if let Some(Ok(Some(w))) = self.wprobe.info().map(wav_check) {
            warn(ui, w);
        }
    }
    fn build(&mut self, _cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let video = check_input(&st.video, VIDEO_EXTS, "input video")?;
        let wav = check_input(&st.wav, &["wav"], "replacement audio")?;
        let vinfo = self.vprobe.require("Video")?;
        let winfo = self.wprobe.require("WAV file")?;
        let warn = wav_check(winfo)?;
        let output = check_output(&st.out_dir, &st.out_name)?;
        if same_file(&video, &output) {
            return Err("Output must be different from the input.".into());
        }
        let mut cmd = replace_audio::build(&ReplaceAudioParams { video, wav, output: output.clone(), audio: st.audio.clone() })?;
        cmd.warnings.extend(warn);
        if vinfo.audio_count() > 1 {
            cmd.warnings.push(format!("The video has {} audio tracks; all of them are replaced by the WAV.", vinfo.audio_count()));
        }
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Replace audio -> {}", st.out_name), cmd, output.clone(), log_path_for(&output), vinfo.duration())],
            ..Default::default()
        })
    }
    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(p) = first_with_ext(paths, VIDEO_EXTS) {
            set_video(&mut self.st, p);
        }
        if let Some(p) = first_with_ext(paths, &["wav"]) {
            self.st.wav = s(p);
        }
    }
    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State { out_dir: s.output_folder.clone(), audio: default_audio(s), ..Default::default() });
        self.vprobe = ProbeSlot::default();
        self.wprobe = ProbeSlot::default();
    }
}
