//! Mode 1 – re-encode a single video.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::chapters::{self, ChapterMode, ManualChapter};
use crate::ui::chapter_creator::{ChapterCreator, Source, Spec};
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
    #[serde(default = "sfx")]
    suffix: String,
    /// "auto" = .mkv for .mkv input, else .mp4 (script behaviour)
    #[serde(default = "auto_container")]
    container: String,
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
    /// With chapters off: create chapters every interval when the input has none.
    auto_chapters_if_none: bool,
    passthrough: Passthrough,
    video: VideoEncode,
    audio: AudioEncode,
    faststart: bool,
}

fn sfx() -> String {
    "_encoded".into()
}
fn auto_container() -> String {
    "auto".into()
}

impl State {
    fn regen_name(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        let p = PathBuf::from(self.input.trim());
        let ext = if self.container == "auto" { reencode::default_output_ext(&p).to_string() } else { self.container.clone() };
        auto_name(&mut self.out_name, &mut self.last_auto, format!("{}{}.{}", stem(&p), self.suffix, ext));
    }

    fn from_settings(s: &Settings) -> Self {
        State {
            input: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: sfx(),
            container: auto_container(),
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
            auto_chapters_if_none: false,
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
    creator: ChapterCreator,
}

impl Mode for ReencodeMode {
    fn key(&self) -> &'static str {
        "reencode"
    }
    fn title(&self) -> &'static str {
        "Re-encode video"
    }
    fn description(&self) -> &'static str {
        "libx264 + AAC with trim, resize, chapters, passthrough, gain/compressor and deinterlacing."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["input", "out_name", "last_auto", "override_duration", "duration"]
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
                st.regen_name();
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
                        ui.colored_label(crate::ui::theme::c().err, "trim exceeds duration");
                    }
                }
            });
            ui.end_row();

            if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) {
                st.regen_name();
            }
            ui.label("Container");
            let before = st.container.clone();
            egui::ComboBox::from_id_salt("m1_cont").selected_text(st.container.clone()).show_ui(ui, |ui| {
                for c in ["auto", "mp4", "mkv", "mov"] {
                    ui.selectable_value(&mut st.container, c.to_string(), if c == "auto" { "auto (MKV in -> MKV, else MP4)" } else { c });
                }
            });
            if before != st.container {
                st.regen_name();
            }
            ui.end_row();
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
                                    ui.colored_label(crate::ui::theme::c().err, e);
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
        let full = if st.override_duration { Some(st.duration) } else { summary.as_ref().map(|v| v.duration_s) };
        let src = Source {
            info: self.probe.info(),
            probing: self.probe.busy(),
            out_dur: full.map(|d| d - st.trim_start - st.trim_end),
            trim_start: st.trim_start,
            batch_note: None,
        };
        let creator = &mut self.creator;
        egui::Grid::new("m1c").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let mut spec = Spec { mode: &mut st.chapter_mode, interval_min: &mut st.chapter_interval_min, manual: &mut st.manual, auto_if_none: &mut st.auto_chapters_if_none };
            creator.grid_rows(ui, "m1", &mut spec, &src);
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
        let mode = if st.chapter_mode == ChapterMode::Off && st.auto_chapters_if_none && !info.has_chapters() { ChapterMode::Auto } else { st.chapter_mode };
        if mode != ChapterMode::Off && out_dur <= 0.0 {
            return Err("Trim duration exceeds video duration".into());
        }
        let chapters = chapters::chapter_input(out_dur, mode, st.chapter_interval_min * 60.0, &st.manual)?;
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
        if mode != st.chapter_mode {
            cmd.warnings.push("The input has no chapters: creating them automatically.".into());
        }
        if mode != ChapterMode::Off && pr.chapters.is_none() {
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
            self.st.regen_name();
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
