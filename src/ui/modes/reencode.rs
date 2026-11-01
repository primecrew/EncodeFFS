//! Mode 1 – re-encode a single video.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::chapters::{self, ChapterMode, ManualChapter};
use ffenc::cmd::common::*;
use ffenc::cmd::reencode::{self, ReencodeParams};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, stem, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    input: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    override_duration: bool,
    duration: f64,
    trim_start: f64,
    trim_end: f64,
    resize: bool,
    resize_height: u32,
    range_convert: bool,
    deinterlace: bool,
    chapter_mode: ChapterMode,
    chapter_interval_min: f64,
    manual: Vec<ManualChapter>,
    passthrough: Passthrough,
    video: VideoEncode,
    audio: AudioEncode,
    faststart: bool,
}

impl State {
    fn from_settings(s: &Settings) -> Self {
        State {
            input: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            override_duration: false,
            duration: 0.0,
            trim_start: s.trim_start,
            trim_end: s.trim_end,
            resize: false,
            resize_height: 720,
            range_convert: false,
            deinterlace: true,
            chapter_mode: ChapterMode::Off,
            chapter_interval_min: s.chapter_interval_min,
            manual: vec![],
            passthrough: Passthrough::None,
            video: default_video(s),
            audio: default_audio(s),
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
pub struct ReencodeMode {
    st: State,
    probe: ProbeSlot,
}

impl Mode for ReencodeMode {
    fn key(&self) -> &'static str {
        "reencode"
    }
    fn title(&self) -> &'static str {
        "1. Re-encode video"
    }
    fn description(&self) -> &'static str {
        "libx264 + AAC with trim, resize, chapters, passthrough, gain/compressor and deinterlacing."
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let st = &mut self.st;
        if self.probe.update(cx.egui, cx.ffprobe, &st.input) {
            if let Some(Ok(v)) = self.probe.info().map(|i| i.video_summary()) {
                st.duration = v.duration_s;
            }
        }
        let summary = self.probe.info().and_then(|i| i.video_summary().ok());
        egui::Grid::new("m1").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Input video", &mut st.input, Pick::File("Video", VIDEO_EXTS), &cx.settings.input_folder) {
                let p = PathBuf::from(st.input.trim());
                let ext = reencode::default_output_ext(&p);
                auto_name(&mut st.out_name, &mut st.last_auto, format!("{}_encoded.{}", stem(&p), ext));
            }
            ui.label("");
            self.probe.status_ui(ui, describe_video);
            ui.end_row();

            ui.label("Duration");
            ui.horizontal(|ui| {
                ui.checkbox(&mut st.override_duration, "Override ffprobe duration");
                if st.override_duration {
                    num_f64(ui, &mut st.duration, 0.0..=1.0e7, 0.1, " s");
                    ui.label(fmt_hms(st.duration));
                }
            });
            ui.end_row();

            ui.label("Trim");
            ui.horizontal(|ui| {
                ui.label("start");
                num_f64(ui, &mut st.trim_start, 0.0..=1.0e7, 0.1, " s");
                ui.label("end");
                num_f64(ui, &mut st.trim_end, 0.0..=1.0e7, 0.1, " s");
                let dur = if st.override_duration { Some(st.duration) } else { summary.as_ref().map(|s| s.duration_s) };
                if let Some(d) = dur {
                    let out = d - st.trim_start - st.trim_end;
                    if out > 0.0 {
                        hint(ui, format!("output {}", fmt_hms(out)));
                    } else {
                        ui.colored_label(egui::Color32::LIGHT_RED, "trim exceeds duration");
                    }
                }
            });
            ui.end_row();

            output_rows(ui, &mut st.out_dir, &mut st.out_name, &cx.settings.output_folder);
            passthrough_ui(ui, "m1_pt", &mut st.passthrough);
        });

        if st.passthrough.encodes_video() {
            section(ui, "Video (libx264)");
            egui::Grid::new("m1v").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                video_encode_ui(ui, "m1", &mut st.video, true);
                ui.label("Resize");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut st.resize, "New vertical resolution");
                    if st.resize {
                        num_u32(ui, &mut st.resize_height, 1..=4320, " px");
                        for h in [480, 720, 1080, 1440, 2160] {
                            if ui.small_button(h.to_string()).clicked() {
                                st.resize_height = h;
                            }
                        }
                        if let Some(s) = &summary {
                            match resize_dims(s.width, s.height, st.resize_height) {
                                Ok((w, h)) => hint(ui, format!("-> {w}x{h}")),
                                Err(e) => {
                                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                                }
                            }
                        }
                    }
                });
                ui.end_row();
                range_ui(ui, &mut st.range_convert);
                ui.label("Deinterlace");
                let interlaced = summary.as_ref().map(|s| s.interlaced).unwrap_or(false);
                ui.vertical(|ui| {
                    ui.add_enabled(interlaced, egui::Checkbox::new(&mut st.deinterlace, "bwdif double-rate deinterlace (recommended)"));
                    if !interlaced {
                        hint(ui, "Only applies when ffprobe reports the input as interlaced.");
                    }
                });
                ui.end_row();
            });
        } else if summary.as_ref().map(|s| s.interlaced).unwrap_or(false) {
            warn(ui, "Input is interlaced, but video passthrough is selected, so no deinterlacing will be applied.");
        }

        if st.passthrough.encodes_audio() {
            section(ui, "Audio (AAC)");
            egui::Grid::new("m1a").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                audio_encode_ui(ui, "m1a", &mut st.audio);
            });
            if summary.as_ref().map(|s| s.audio_count > 1).unwrap_or(false) && st.audio.filter_chain().is_some() {
                hint(ui, "Gain/compressor is applied to every audio track separately.");
            }
        }

        section(ui, "Chapters & output");
        egui::Grid::new("m1c").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Chapters");
            egui::ComboBox::from_id_salt("m1_ch").selected_text(st.chapter_mode.label()).show_ui(ui, |ui| {
                for m in ChapterMode::ALL {
                    ui.selectable_value(&mut st.chapter_mode, m, m.label());
                }
            });
            ui.end_row();
            if matches!(st.chapter_mode, ChapterMode::Auto | ChapterMode::Both) {
                ui.label("Interval");
                num_f64(ui, &mut st.chapter_interval_min, 0.1..=600.0, 0.1, " min");
                ui.end_row();
            }
            if matches!(st.chapter_mode, ChapterMode::Manual | ChapterMode::Both) {
                ui.label("Manual chapters");
                ui.vertical(|ui| {
                    hint(ui, "hh:mm:ss relative to the trimmed output");
                    let mut remove = None;
                    for (i, c) in st.manual.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut c.time).hint_text("00:15:30").desired_width(80.0));
                            ui.add(egui::TextEdit::singleline(&mut c.title).hint_text(format!("Chapter {}", i + 1)).desired_width(220.0));
                            if ui.small_button("✖").clicked() {
                                remove = Some(i);
                            }
                            if !c.time.trim().is_empty() {
                                if let Err(e) = chapters::parse_hms(&c.time) {
                                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                                }
                            }
                        });
                    }
                    if let Some(i) = remove {
                        st.manual.remove(i);
                    }
                    if ui.button("+ Add chapter").clicked() {
                        st.manual.push(ManualChapter::default());
                    }
                });
                ui.end_row();
            }
            let mp4 = reencode::is_mp4_like(&PathBuf::from(&st.out_name));
            faststart_ui(ui, &mut st.faststart, mp4);
        });
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        let input = check_input(&st.input, VIDEO_EXTS, "input video")?;
        let info = self.probe.require("Input video")?;
        let v = info.video_summary()?;
        let output = check_output(&st.out_dir, &st.out_name)?;
        if same_file(&input, &output) {
            return Err("Output must be different from the input.".into());
        }
        let duration = if st.override_duration { st.duration } else { v.duration_s };
        let out_dur = duration - st.trim_start - st.trim_end;
        let resize = if st.resize && st.passthrough.encodes_video() { Some(resize_dims(v.width, v.height, st.resize_height)?) } else { None };
        let chapters = if st.chapter_mode != ChapterMode::Off {
            if out_dur <= 0.0 {
                return Err("Trim duration exceeds video duration".into());
            }
            let list = chapters::build_chapters(out_dur, st.chapter_mode, st.chapter_interval_min * 60.0, &st.manual)?;
            if list.is_empty() {
                None
            } else {
                let path = std::env::temp_dir().join(format!("ffenc_chapters_{}.txt", ffenc::util::unique_id()));
                Some((path, chapters::ffmetadata(&list)))
            }
        } else {
            None
        };
        let pr = ReencodeParams {
            input: input.clone(),
            output: output.clone(),
            duration_s: duration,
            trim_start: st.trim_start,
            trim_end: st.trim_end,
            passthrough: st.passthrough,
            video: st.video.clone(),
            audio: st.audio.clone(),
            range_convert: st.range_convert,
            resize,
            deinterlace: st.deinterlace && v.interlaced,
            chapters,
            faststart: st.faststart,
            subtitle_streams: info.subtitle_refs(),
            has_attachments: info.has_attachments(),
        };
        let mut cmd = reencode::build(&pr)?;
        if st.chapter_mode != ChapterMode::Off && pr.chapters.is_none() {
            cmd.warnings.push("No chapters generated.".into());
        }
        if let Some((_, text)) = &pr.chapters {
            cmd.warnings.push(format!("Chapters:\n{}", text.trim_end()));
        }
        let _ = cx;
        Ok(Plan {
            jobs: vec![JobSpec::new(
                format!("Re-encode {}", input.file_name().unwrap_or_default().to_string_lossy()),
                cmd,
                output.clone(),
                log_path_for(&output),
                Some(pr.output_duration()),
            )],
            ..Default::default()
        })
    }

    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(p) = first_with_ext(paths, VIDEO_EXTS) {
            self.st.input = s(p);
            let ext = reencode::default_output_ext(p);
            auto_name(&mut self.st.out_name, &mut self.st.last_auto, format!("{}_encoded.{}", stem(p), ext));
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }

    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        self.probe = ProbeSlot::default();
    }
}
