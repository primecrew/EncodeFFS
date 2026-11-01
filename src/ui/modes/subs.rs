//! Mode 8 – copy subtitle tracks from one video into another.

use std::collections::BTreeSet;
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::common::VideoEncode;
use ffenc::cmd::subs::{self, Mp4SubHandling, SubsParams};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{stem, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    source: String,
    dest: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    container: String,
    keep_existing: bool,
    copy_only: bool,
    video: VideoEncode,
    audio_bitrate_k: u32,
    range_convert: bool,
    mp4_handling: Mp4SubHandling,
    faststart: bool,
}
impl State {
    fn from_settings(s: &Settings) -> Self {
        State {
            source: String::new(),
            dest: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            container: "mkv".into(),
            keep_existing: true,
            copy_only: true,
            video: default_video(s),
            audio_bitrate_k: s.audio_bitrate_k,
            range_convert: false,
            mp4_handling: Mp4SubHandling::ConvertMovText,
            faststart: true,
        }
    }
    fn regen_name(&mut self) {
        if self.dest.trim().is_empty() {
            return;
        }
        let p = PathBuf::from(self.dest.trim());
        auto_name(&mut self.out_name, &mut self.last_auto, format!("{}_with_subs.{}", stem(&p), self.container));
    }
}
impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

#[derive(Default)]
pub struct SubsMode {
    st: State,
    sprobe: ProbeSlot,
    dprobe: ProbeSlot,
    selected: BTreeSet<u32>,
}

impl Mode for SubsMode {
    fn key(&self) -> &'static str {
        "subs"
    }
    fn title(&self) -> &'static str {
        "8. Copy subtitle tracks"
    }
    fn description(&self) -> &'static str {
        "Pick subtitle tracks from a source video and mux them into a destination video."
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        if self.sprobe.update(cx.egui, cx.ffprobe, &st.source) {
            self.selected = self.sprobe.info().map(|i| i.subtitle_infos().iter().map(|s| s.index).collect()).unwrap_or_default();
        }
        self.dprobe.update(cx.egui, cx.ffprobe, &st.dest);
        egui::Grid::new("m8").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            path_row(ui, "Subtitle source", &mut st.source, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder);
            ui.label("Tracks to copy");
            ui.vertical(|ui| {
                if self.sprobe.busy() {
                    ui.spinner();
                } else if let Some(i) = self.sprobe.info() {
                    let subs = i.subtitle_infos();
                    if subs.is_empty() {
                        ui.colored_label(egui::Color32::LIGHT_RED, "No subtitle streams in the source file.");
                    }
                    for sub in subs {
                        let mut on = self.selected.contains(&sub.index);
                        if ui.checkbox(&mut on, sub.describe()).changed() {
                            if on {
                                self.selected.insert(sub.index);
                            } else {
                                self.selected.remove(&sub.index);
                            }
                        }
                    }
                } else {
                    self.sprobe.status_ui(ui, |_| String::new());
                }
            });
            ui.end_row();
            if path_row(ui, "Destination video", &mut st.dest, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder) {
                st.regen_name();
            }
            ui.label("Existing subtitles");
            ui.vertical(|ui| {
                if let Some(i) = self.dprobe.info() {
                    let subs = i.subtitle_infos();
                    if subs.is_empty() {
                        ui.label("No subtitle streams detected in the destination file.");
                    }
                    for sub in subs {
                        ui.label(sub.describe());
                    }
                } else {
                    self.dprobe.status_ui(ui, |_| String::new());
                }
                ui.checkbox(&mut st.keep_existing, "Keep subtitle tracks already in the destination");
            });
            ui.end_row();
            ui.label("Container");
            ui.horizontal(|ui| {
                let before = st.container.clone();
                ui.radio_value(&mut st.container, "mkv".to_string(), "MKV (recommended)");
                ui.radio_value(&mut st.container, "mp4".to_string(), "MP4");
                if before != st.container {
                    let p = PathBuf::from(st.out_name.trim());
                    if !st.out_name.trim().is_empty() {
                        let new = format!("{}.{}", stem(&p), st.container);
                        if st.out_name == st.last_auto {
                            st.last_auto = new.clone();
                        }
                        st.out_name = new;
                    }
                }
            });
            ui.end_row();
            output_rows(ui, &mut st.out_dir, &mut st.out_name, &cx.settings.output_folder);
            ui.label("Video/audio");
            ui.checkbox(&mut st.copy_only, "Copy without re-encoding (recommended)");
            ui.end_row();
            if !st.copy_only {
                video_encode_ui(ui, "m8", &mut st.video, false);
                bitrate_ui(ui, "AAC bitrate", &mut st.audio_bitrate_k);
                range_ui(ui, &mut st.range_convert);
            }
            if st.container == "mp4" {
                ui.label("MP4 subtitles");
                egui::ComboBox::from_id_salt("m8_mp4").selected_text(match st.mp4_handling {
                    Mp4SubHandling::ConvertMovText => "Convert to mov_text",
                    Mp4SubHandling::TryCopy => "Try copying as-is",
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut st.mp4_handling, Mp4SubHandling::ConvertMovText, "Convert to mov_text");
                    ui.selectable_value(&mut st.mp4_handling, Mp4SubHandling::TryCopy, "Try copying as-is");
                });
                ui.end_row();
                faststart_ui(ui, &mut st.faststart, true);
            }
        });
        // live MP4 compatibility warning
        if st.container == "mp4" {
            if let Some(i) = self.sprobe.info() {
                let src = i.subtitle_infos();
                let mut involved: Vec<_> = src.iter().filter(|s| self.selected.contains(&s.index)).collect();
                let dsubs = self.dprobe.info().map(|d| d.subtitle_infos()).unwrap_or_default();
                if st.keep_existing {
                    involved.extend(dsubs.iter());
                }
                if let (_, Some(w)) = subs::subtitle_codec(true, &involved, st.mp4_handling) {
                    warn(ui, w);
                }
            }
        }
    }
    fn build(&mut self, _cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let source = check_input(&st.source, VIDEO_EXTS, "subtitle source video")?;
        let dest = check_input(&st.dest, VIDEO_EXTS, "destination video")?;
        let si = self.sprobe.require("Subtitle source")?;
        let di = self.dprobe.require("Destination video")?;
        let mut name = st.out_name.trim().to_string();
        let ext = ffenc::util::ext_lower(std::path::Path::new(&name));
        if ext != "mkv" && ext != "mp4" {
            name = format!("{}.{}", stem(std::path::Path::new(&name)), st.container);
        }
        let output = check_output(&st.out_dir, &name)?;
        if same_file(&dest, &output) || same_file(&source, &output) {
            return Err("Output must be different from the inputs.".into());
        }
        let pr = SubsParams {
            source,
            destination: dest,
            output: output.clone(),
            source_streams: si.subtitle_infos(),
            selected: self.selected.iter().copied().collect(),
            dest_streams: di.subtitle_infos(),
            keep_existing: st.keep_existing,
            copy_only: st.copy_only,
            video: VideoEncode { tune: None, ..st.video.clone() },
            audio_bitrate_k: st.audio_bitrate_k,
            range_convert: st.range_convert,
            mp4_handling: st.mp4_handling,
            faststart: st.faststart,
        };
        let cmd = subs::build(&pr)?;
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Subtitles -> {name}"), cmd, output.clone(), log_path_for(&output), di.duration())],
            ..Default::default()
        })
    }
    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        let vids: Vec<&PathBuf> = paths.iter().filter(|p| ffenc::util::has_ext(p, VIDEO_EXTS)).collect();
        match vids.as_slice() {
            [a, b, ..] => {
                self.st.source = s(a);
                self.st.dest = s(b);
                self.st.regen_name();
            }
            [a] => {
                if self.st.source.trim().is_empty() {
                    self.st.source = s(a);
                } else {
                    self.st.dest = s(a);
                    self.st.regen_name();
                }
            }
            [] => {}
        }
    }
    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        self.sprobe = ProbeSlot::default();
        self.dprobe = ProbeSlot::default();
        self.selected.clear();
    }
}
