//! Mode 7 – batch re-encode every supported video in a folder.

use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::batch::{self, BatchInput, BatchSettings};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, has_ext, VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    in_dir: String,
    out_dir: String,
    bs: BatchSettings,
}

struct Row {
    path: PathBuf,
    info: Result<MediaInfo, String>,
    include: bool,
}

#[derive(Default)]
pub struct BatchMode {
    st: State,
    rows: Vec<Row>,
    scanned_dir: String,
    scan: Option<Bg<Result<Vec<(PathBuf, Result<MediaInfo, String>)>, String>>>,
}

/// Supported video files in a folder, sorted by name (script: Get-VideoFilesInFolder).
pub fn list_videos(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && has_ext(p, VIDEO_EXTS))
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(v)
}

impl BatchMode {
    fn start_scan(&mut self, cx: &Ctx) {
        let Some(fp) = cx.ffprobe.map(|p| p.to_path_buf()) else {
            self.rows.clear();
            return;
        };
        let dir = PathBuf::from(self.st.in_dir.trim());
        self.scanned_dir = self.st.in_dir.clone();
        self.rows.clear();
        self.scan = Some(Bg::spawn(cx.egui, move || {
            let files = list_videos(&dir)?;
            Ok(files.into_iter().map(|f| {
                let r = probe::probe(&fp, &f);
                (f, r)
            }).collect())
        }));
    }
}

impl Mode for BatchMode {
    fn key(&self) -> &'static str {
        "batch"
    }
    fn title(&self) -> &'static str {
        "Batch re-encode folder"
    }
    fn description(&self) -> &'static str {
        "Re-encode every .mp4/.mkv/.avi/.mov/.avs in a folder with shared settings."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["in_dir"]
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        if let Some(t) = &self.scan {
            if let Some(r) = t.poll() {
                self.scan = None;
                match r {
                    Ok(list) => self.rows = list.into_iter().map(|(path, info)| Row { include: info.is_ok(), path, info }).collect(),
                    Err(e) => {
                        self.rows.clear();
                        self.scanned_dir = format!("ERROR: {e}");
                    }
                }
            }
        }
        let mut rescan = false;
        egui::Grid::new("m7").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Input folder", &mut self.st.in_dir, Pick::Folder, &cx.settings.input_folder) {
                rescan = PathBuf::from(self.st.in_dir.trim()).is_dir();
            }
            ui.label("");
            ui.horizontal(|ui| {
                if ui.button("Scan folder").clicked() {
                    rescan = true;
                }
                if self.scan.is_some() {
                    ui.spinner();
                    ui.label("probing files…");
                } else if self.scanned_dir.starts_with("ERROR") {
                    ui.colored_label(crate::ui::theme::c().err, &self.scanned_dir);
                } else if !self.rows.is_empty() {
                    ui.label(format!("{} file(s), {} selected", self.rows.len(), self.rows.iter().filter(|r| r.include).count()));
                }
            });
            ui.end_row();
            path_row_nodrop(ui, "Output folder", &mut self.st.out_dir, Pick::Folder, &cx.settings.output_folder);
        });
        if rescan {
            self.start_scan(cx);
        }
        if cx.ffprobe.is_none() {
            warn(ui, "FFprobe not found – set the FFmpeg folder in Settings.");
        }

        if !self.rows.is_empty() {
            crate::ui::layout::section(ui, "batch.files", egui::CollapsingHeader::new(format!("Files ({})", self.rows.len())), true, |ui| {
                egui::ScrollArea::vertical().max_height(180.0).id_salt("m7files").show(ui, |ui| {
                    egui::Grid::new("m7rows").striped(true).num_columns(3).show(ui, |ui| {
                        for r in &mut self.rows {
                            ui.add_enabled(r.info.is_ok(), egui::Checkbox::new(&mut r.include, r.path.file_name().unwrap_or_default().to_string_lossy()));
                            match r.info.as_ref().map_err(|e| e.clone()).and_then(|i| i.video_summary()) {
                                Ok(v) => {
                                    let ch = r.info.as_ref().map(|i| i.chapters.len()).unwrap_or(0);
                                    let ch = if ch == 0 { "no chapters".to_string() } else { format!("{ch} chapters") };
                                    ui.label(format!("{} · {}x{} · {} audio · {ch}", fmt_hms(v.duration_s), v.width, v.height, v.audio_count));
                                    if v.interlaced {
                                        ui.colored_label(crate::ui::theme::c().warn, format!("interlaced ({})", v.field_order.unwrap_or_default()));
                                    } else {
                                        ui.label("");
                                    }
                                }
                                Err(e) => {
                                    ui.colored_label(crate::ui::theme::c().err, e);
                                    ui.label("");
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        }

        let without = self.rows.iter().filter(|r| r.include && r.info.as_ref().map(|i| !i.has_chapters()).unwrap_or(false)).count();
        let bs = &mut self.st.bs;
        let any_interlaced = self.rows.iter().any(|r| r.include && r.info.as_ref().ok().and_then(|i| i.video_summary().ok()).map(|v| v.interlaced).unwrap_or(false));
        section(ui, "Shared settings");
        egui::Grid::new("m7s").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::common::row_label(ui, "Container");
            ui.horizontal(|ui| {
                ui.radio_value(&mut bs.container, "mp4".to_string(), "MP4");
                ui.radio_value(&mut bs.container, "mkv".to_string(), "MKV");
            });
            ui.end_row();
            crate::ui::common::row_label(ui, "Name suffix");
            ui.text_edit_singleline(&mut bs.suffix);
            ui.end_row();
            crate::ui::common::row_label(ui, "Trim");
            ui.horizontal(|ui| {
                ui.label("start");
                num_f64(ui, &mut bs.trim_start, 0.0..=1.0e7, 0.1, " s");
                ui.label("end");
                num_f64(ui, &mut bs.trim_end, 0.0..=1.0e7, 0.1, " s");
            });
            ui.end_row();
            passthrough_ui(ui, "m7_pt", &mut bs.passthrough);
            crate::ui::common::row_label(ui, "Chapters");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut bs.auto_chapters_if_none, "Create chapters for files without any, every");
                    ui.add_enabled_ui(bs.auto_chapters_if_none, |ui| num_f64(ui, &mut bs.chapter_interval_min, 0.1..=600.0, 0.1, " min"));
                });
                if !self.rows.is_empty() {
                    hint(ui, format!("{without} selected file(s) have no chapters; files with chapters keep theirs."));
                }
            });
            ui.end_row();
        });
        if bs.passthrough.encodes_video() {
            section(ui, "Video (libx264)");
            egui::Grid::new("m7v").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                video_encode_ui(ui, "m7", &mut bs.video, true);
                range_ui(ui, &mut bs.range_convert);
                crate::ui::common::row_label(ui, "Deinterlace");
                ui.vertical(|ui| {
                    ui.checkbox(&mut bs.deinterlace, "bwdif double-rate deinterlace for files detected as interlaced");
                    if !any_interlaced && !self.rows.is_empty() {
                        hint(ui, "FFprobe did not detect interlaced video in the selected files.");
                    }
                });
                ui.end_row();
            });
        } else if any_interlaced {
            warn(ui, "Interlaced video detected, but video passthrough is selected, so no deinterlacing will be applied.");
        }
        if bs.passthrough.encodes_audio() {
            section(ui, "Audio (AAC)");
            egui::Grid::new("m7a").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                audio_encode_ui(ui, "m7a", &mut bs.audio);
            });
        }
        egui::Grid::new("m7f").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            faststart_ui(ui, &mut bs.faststart, bs.container == "mp4");
        });
    }

    fn build(&mut self, _cx: &Ctx) -> Result<Plan, String> {
        if self.scan.is_some() {
            return Err("Still scanning the folder.".into());
        }
        if self.rows.is_empty() {
            return Err(format!("No supported video files found in: {} (press Scan folder)", self.st.in_dir));
        }
        let out_dir = PathBuf::from(self.st.out_dir.trim());
        if self.st.out_dir.trim().is_empty() {
            return Err("Choose an output folder.".into());
        }
        let mut inputs = vec![];
        for r in self.rows.iter().filter(|r| r.include) {
            let info = r.info.as_ref().map_err(|e| format!("FFprobe error on '{}': {e}", r.path.display()))?;
            let v = info.video_summary().map_err(|e| format!("FFprobe error on '{}': {e}", r.path.display()))?;
            inputs.push(BatchInput {
                path: r.path.clone(),
                duration_s: v.duration_s,
                field_order: v.field_order.clone(),
                subtitle_streams: info.subtitle_refs(),
                has_attachments: info.has_attachments(),
                has_chapters: info.has_chapters(),
            });
        }
        let jobs = batch::plan(&inputs, &self.st.bs, &out_dir)?;
        let n = jobs.len();
        let specs = jobs
            .into_iter()
            .map(|j| JobSpec::new(format!("Batch: {}", j.input.file_name().unwrap_or_default().to_string_lossy()), j.cmd, j.output, j.log_file, Some(j.out_duration)))
            .collect();
        Ok(Plan { jobs: specs, group: Some(("Batch re-encode".into(), false, 0)), notes: vec![format!("Prepared {n} batch encoding job(s).")] })
    }

    fn wants_folders(&self) -> bool {
        true
    }
    fn on_drop(&mut self, paths: &[PathBuf], cx: &Ctx) {
        if let Some(p) = paths.first() {
            let dir = if p.is_dir() { p.clone() } else { p.parent().map(|d| d.to_path_buf()).unwrap_or_default() };
            self.st.in_dir = s(&dir);
            self.start_scan(cx);
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        let bs = BatchSettings {
            trim_start: s.trim_start,
            trim_end: s.trim_end,
            video: default_video(s),
            audio: default_audio(s),
            ..Default::default()
        };
        self.st = restore(v, State { in_dir: s.input_folder.clone(), out_dir: s.output_folder.clone(), bs });
        if self.st.in_dir.trim().is_empty() {
            self.st.in_dir = s.input_folder.clone();
        }
        self.rows.clear();
        self.scanned_dir.clear();
    }
}
