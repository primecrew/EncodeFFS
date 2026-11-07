//! "Custom" preset type: compose a pipeline from video / audio / subtitle actions.

use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::chapters::ChapterMode;
use crate::ui::chapter_creator::{ChapterCreator, Source, Spec};
use ffenc::cmd::custom::{self, AudioAction, CustomOptions, SubAction, VideoAction, CONTAINERS};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{find_companion, has_ext, ANY_AUDIO_EXTS, ANY_VIDEO_EXTS, SUB_EXTS};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    /// File (single) or folder (batch) – per run.
    input: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    /// Replace / add-track audio file – per run (batch: matched by name).
    audio_file: String,
    /// Subtitle file to import – per run (batch: matched by name).
    sub_file: String,
    #[serde(flatten)]
    o: CustomOptions,
}

pub const RUN_FIELDS: &[&str] = &["input", "out_name", "last_auto", "audio_file", "sub_file"];

struct Row {
    path: PathBuf,
    info: Result<MediaInfo, String>,
    include: bool,
}

#[derive(Default)]
pub struct CustomMode {
    st: State,
    probe: ProbeSlot,
    creator: ChapterCreator,
    rows: Vec<Row>,
    scanned: String,
    scan: Option<Bg<Result<Vec<(PathBuf, Result<MediaInfo, String>)>, String>>>,
}

fn input_exts() -> Vec<&'static str> {
    ANY_VIDEO_EXTS.iter().chain(ANY_AUDIO_EXTS.iter()).copied().collect()
}

fn list_videos(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && has_ext(p, ANY_VIDEO_EXTS))
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(v)
}

impl State {
    fn regen_name(&mut self) {
        if self.o.batch || self.input.trim().is_empty() {
            return;
        }
        let n = self.o.output_name(Path::new(self.input.trim()));
        auto_name(&mut self.out_name, &mut self.last_auto, n);
    }
}

impl CustomMode {
    fn start_scan(&mut self, cx: &Ctx) {
        self.rows.clear();
        let Some(fp) = cx.ffprobe.map(|p| p.to_path_buf()) else { return };
        let dir = PathBuf::from(self.st.input.trim());
        self.scanned = self.st.input.clone();
        self.scan = Some(Bg::spawn(cx.egui, move || Ok(list_videos(&dir)?.into_iter().map(|f| {
            let r = probe::probe(&fp, &f);
            (f, r)
        }).collect())));
    }

    fn job(&self, input: &Path, info: &MediaInfo, output: PathBuf, audio_file: Option<PathBuf>, sub_file: Option<PathBuf>) -> Result<JobSpec, String> {
        let pr = self.st.o.params(input, &output, info, audio_file, sub_file)?;
        let cmd = custom::build(&pr)?;
        let dur = pr.output_duration();
        Ok(JobSpec::new(format!("Custom: {}", input.file_name().unwrap_or_default().to_string_lossy()), cmd, output.clone(), log_path_for(&output), dur))
    }
}

impl Mode for CustomMode {
    fn key(&self) -> &'static str {
        "custom"
    }
    fn title(&self) -> &'static str {
        "Custom pipeline"
    }
    fn description(&self) -> &'static str {
        "Compose your own: video / audio / subtitle actions, trim, chapters, container, extra ffmpeg args."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        RUN_FIELDS
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        if let Some(t) = &self.scan {
            if let Some(r) = t.poll() {
                self.scan = None;
                match r {
                    Ok(l) => self.rows = l.into_iter().map(|(path, info)| Row { include: info.is_ok(), path, info }).collect(),
                    Err(e) => self.scanned = format!("ERROR: {e}"),
                }
            }
        }
        let batch = self.st.o.batch;
        if !batch {
            self.probe.update(cx.egui, cx.ffprobe, &self.st.input);
        }
        let mut rescan = false;
        let st = &mut self.st;
        section(ui, "Input & output");
        egui::Grid::new("cu_io").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::common::row_label(ui, "Input type");
            ui.horizontal(|ui| {
                let a = ui.radio_value(&mut st.o.batch, false, "Single file").changed();
                let b = ui.radio_value(&mut st.o.batch, true, "Every video in a folder").changed();
                if a || b {
                    st.input.clear();
                    st.out_name.clear();
                    st.last_auto.clear();
                }
            });
            ui.end_row();
            if st.o.batch {
                if path_row(ui, "Input folder", &mut st.input, Pick::Folder, &cx.settings.input_folder) {
                    rescan = Path::new(st.input.trim()).is_dir();
                }
                ui.label("");
                ui.horizontal(|ui| {
                    if ui.button("Scan folder").clicked() {
                        rescan = true;
                    }
                    if self.scan.is_some() {
                        ui.spinner();
                        ui.label("probing files…");
                    } else if self.scanned.starts_with("ERROR") {
                        ui.colored_label(crate::ui::theme::c().err, &self.scanned);
                    } else if !self.rows.is_empty() {
                        ui.label(format!("{} file(s), {} selected", self.rows.len(), self.rows.iter().filter(|r| r.include).count()));
                    }
                });
                ui.end_row();
                path_row_nodrop(ui, "Output folder", &mut st.out_dir, Pick::Folder, &cx.settings.output_folder);
                crate::ui::common::row_label(ui, "Name suffix");
                ui.text_edit_singleline(&mut st.o.suffix);
                ui.end_row();
            } else {
                let exts = input_exts();
                if path_row(ui, "Input file", &mut st.input, Pick::File("Media", &exts), &cx.settings.input_folder) {
                    st.regen_name();
                }
                ui.label("");
                self.probe.status_ui(ui, |i| if i.video().is_some() { describe_video(i) } else { describe_audio(i) });
                ui.end_row();
                if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.o.suffix, &cx.settings.output_folder) {
                    st.regen_name();
                }
            }
            crate::ui::common::row_label(ui, "Container");
            let before = st.o.container.clone();
            egui::ComboBox::from_id_salt("cu_cont").selected_text(st.o.container.clone()).show_ui(ui, |ui| {
                for c in CONTAINERS {
                    ui.selectable_value(&mut st.o.container, c.to_string(), *c);
                }
            });
            if before != st.o.container {
                st.regen_name();
            }
            ui.end_row();
            crate::ui::common::row_label(ui, "Trim");
            ui.horizontal(|ui| {
                ui.label("start");
                num_f64(ui, &mut st.o.trim_start, 0.0..=1.0e7, 0.1, " s");
                ui.label("end");
                num_f64(ui, &mut st.o.trim_end, 0.0..=1.0e7, 0.1, " s");
            });
            ui.end_row();
        });
        if st.o.batch && !self.rows.is_empty() {
            crate::ui::layout::section(ui, "custom.files", egui::CollapsingHeader::new(format!("Files ({})", self.rows.len())), false, |ui| {
                egui::ScrollArea::vertical().max_height(160.0).id_salt("cu_files").show(ui, |ui| {
                    for r in &mut self.rows {
                        ui.horizontal(|ui| {
                            ui.add_enabled(r.info.is_ok(), egui::Checkbox::new(&mut r.include, r.path.file_name().unwrap_or_default().to_string_lossy()));
                            match &r.info {
                                Ok(i) => hint(ui, describe_video(i)),
                                Err(e) => {
                                    ui.colored_label(crate::ui::theme::c().err, e);
                                }
                            }
                        });
                    }
                });
            });
        }

        let o = &mut st.o;
        section(ui, "Video");
        egui::Grid::new("cu_v").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::common::row_label(ui, "Video");
            egui::ComboBox::from_id_salt("cu_va").width(260.0).selected_text(o.video.label()).show_ui(ui, |ui| {
                for v in VideoAction::ALL {
                    ui.selectable_value(&mut o.video, v, v.label());
                }
            });
            ui.end_row();
            if o.video == VideoAction::X264 {
                crate::ui::codecs_ui::video_codec_ui(ui, "cu", &mut o.video_encode, cx.encoders.as_ref(), None);
                crate::ui::common::row_label(ui, "Resize");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut o.resize, "New vertical resolution");
                    if o.resize {
                        num_u32(ui, &mut o.resize_height, 1..=4320, " px");
                        for h in [480, 720, 1080, 1440, 2160] {
                            if ui.small_button(h.to_string()).clicked() {
                                o.resize_height = h;
                            }
                        }
                    }
                });
                ui.end_row();
                range_ui(ui, &mut o.range_convert);
                crate::ui::common::row_label(ui, "Deinterlace");
                ui.checkbox(&mut o.deinterlace, "bwdif double-rate, only when ffprobe reports interlaced video");
                ui.end_row();
            }
        });

        section(ui, "Audio");
        egui::Grid::new("cu_a").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::common::row_label(ui, "Audio");
            egui::ComboBox::from_id_salt("cu_aa").width(300.0).selected_text(o.audio.label()).show_ui(ui, |ui| {
                for v in AudioAction::ALL {
                    ui.selectable_value(&mut o.audio, v, v.label());
                }
            });
            ui.end_row();
            if o.audio.needs_file() {
                if o.batch {
                    crate::ui::common::row_label(ui, "Audio file");
                    hint(ui, "Batch: uses the file with the same name next to each video (.wav, .flac, .m4a, .mp3 …); videos without one are skipped.");
                    ui.end_row();
                } else {
                    path_row(ui, "Audio file", &mut st.audio_file, Pick::File("Audio", ANY_AUDIO_EXTS), &cx.settings.input_folder);
                }
            }
            if matches!(o.audio, AudioAction::AddTrack | AudioAction::Mix | AudioAction::Replace) {
                crate::ui::common::row_label(ui, "Track title");
                ui.add(egui::TextEdit::singleline(&mut o.track_title).hint_text(cx.settings.commentary_name.as_str()).desired_width(240.0));
                ui.end_row();
            }
            if o.audio.encodes() {
                audio_encode_ui(ui, "cu_ae", &mut o.audio_encode);
            }
        });

        let csrc = if o.batch {
            let ok: Vec<&MediaInfo> = self.rows.iter().filter_map(|r| r.info.as_ref().ok()).collect();
            let with = ok.iter().filter(|i| i.has_chapters()).count();
            Source {
                batch_note: Some(if ok.is_empty() { "batch: scan a folder to check for chapters".into() } else { format!("batch: {with} of {} file(s) already have chapters", ok.len()) }),
                trim_start: o.trim_start,
                ..Default::default()
            }
        } else {
            Source {
                info: self.probe.info(),
                probing: self.probe.busy(),
                out_dur: self.probe.info().and_then(|i| i.any_duration()).map(|d| d - o.trim_start - o.trim_end),
                trim_start: o.trim_start,
                batch_note: None,
            }
        };
        section(ui, "Subtitles, chapters & extras");
        egui::Grid::new("cu_s").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::common::row_label(ui, "Subtitles");
            egui::ComboBox::from_id_salt("cu_sa").width(300.0).selected_text(o.subs.label()).show_ui(ui, |ui| {
                for v in SubAction::ALL {
                    ui.selectable_value(&mut o.subs, v, v.label());
                }
            });
            ui.end_row();
            if o.subs == SubAction::Import {
                if o.batch {
                    crate::ui::common::row_label(ui, "Subtitle file");
                    hint(ui, "Batch: uses the .srt/.ass/.vtt with the same name next to each video; videos without one are skipped.");
                    ui.end_row();
                } else {
                    path_row(ui, "Subtitle file", &mut st.sub_file, Pick::File("Subtitles", SUB_EXTS), &cx.settings.input_folder);
                }
                ui.label("");
                ui.checkbox(&mut o.keep_existing_subs, "Keep the input's own subtitle tracks too");
                ui.end_row();
            }
            {
                let mut spec = Spec { mode: &mut o.chapter_mode, interval_min: &mut o.chapter_interval_min, manual: &mut o.manual, auto_if_none: &mut o.auto_chapters_if_none };
                self.creator.grid_rows(ui, "cu", &mut spec, &csrc);
            }
            if o.chapter_mode == ChapterMode::Off {
                ui.label("");
                ui.add_enabled_ui(!o.auto_chapters_if_none || o.batch, |ui| ui.checkbox(&mut o.drop_chapters, "Remove the input's own chapters"));
                ui.end_row();
            }
            let mp4 = matches!(o.container.as_str(), "mp4" | "mov" | "m4v" | "m4a");
            faststart_ui(ui, &mut o.faststart, mp4);
            crate::ui::common::row_label_tip(ui, "Extra input options", "Additional FFmpeg options added before the input file (-i), exactly as typed (advanced).");
            ui.vertical(|ui| {
                ui.add(egui::TextEdit::singleline(&mut o.input_args).hint_text("e.g. -hwaccel auto").desired_width(f32::INFINITY));
                if let Err(e) = custom::split_args(&o.input_args) {
                    ui.colored_label(crate::ui::theme::c().err, e);
                }
            });
            ui.end_row();
            crate::ui::common::row_label_tip(ui, "Extra output options", "Additional FFmpeg options added before the output file, exactly as typed (advanced).");
            ui.vertical(|ui| {
                ui.add(egui::TextEdit::singleline(&mut o.output_args).hint_text("e.g. -metadata comment=\"my game\" -af loudnorm").desired_width(f32::INFINITY));
                if let Err(e) = custom::split_args(&o.output_args) {
                    ui.colored_label(crate::ui::theme::c().err, e);
                }
                hint(ui, "Inserted just before the output file; use quotes for values with spaces. Shown in the preview.");
            });
            ui.end_row();
        });
        if rescan {
            self.start_scan(cx);
        }
        if cx.ffprobe.is_none() {
            warn(ui, "FFprobe not found – set the FFmpeg folder first.");
        }
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        if self.st.o.video == VideoAction::X264 {
            crate::ui::codecs_ui::check_available(cx.encoders.as_ref(), Some(&self.st.o.video_encode), None)?;
        }
        let st = &self.st;
        if st.o.video == VideoAction::X264 {
            ffenc::cmd::common::parse_output_fps(&st.o.video_encode.output_fps)?;
        }
        if st.out_dir.trim().is_empty() {
            return Err("Choose an output folder.".into());
        }
        if !st.o.batch {
            let input = check_input(&st.input, &input_exts(), "input file")?;
            let info = self.probe.require("Input file")?;
            let output = check_output(&st.out_dir, &st.out_name)?;
            if same_file(&input, &output) {
                return Err("Output must be different from the input.".into());
            }
            let opt = |s: &str| (!s.trim().is_empty()).then(|| PathBuf::from(s.trim()));
            let af = if st.o.audio.needs_file() { opt(&st.audio_file) } else { None };
            if let Some(f) = &af {
                if !f.is_file() {
                    return Err(format!("Audio file not found: {}", f.display()));
                }
            }
            let sf = if st.o.subs == SubAction::Import { opt(&st.sub_file) } else { None };
            if let Some(f) = &sf {
                if !f.is_file() {
                    return Err(format!("Subtitle file not found: {}", f.display()));
                }
            }
            let job = self.job(&input, info, output, af, sf)?;
            return Ok(Plan { jobs: vec![job], ..Default::default() });
        }
        if self.scan.is_some() {
            return Err("Still scanning the folder.".into());
        }
        if self.rows.is_empty() {
            return Err(format!("No video files found in: {} (press Scan folder)", st.input));
        }
        let out_dir = PathBuf::from(st.out_dir.trim());
        let mut jobs = vec![];
        let mut notes = vec![];
        for r in self.rows.iter().filter(|r| r.include) {
            let Ok(info) = &r.info else { continue };
            let output = out_dir.join(st.o.output_name(&r.path));
            if same_file(&r.path, &output) {
                notes.push(format!("Skipped {}: output would overwrite the input", r.path.display()));
                continue;
            }
            let af = if st.o.audio.needs_file() {
                match find_companion(&r.path, ANY_AUDIO_EXTS) {
                    Some(f) => Some(f),
                    None => {
                        notes.push(format!("Skipped {}: no audio file with the same name", r.path.display()));
                        continue;
                    }
                }
            } else {
                None
            };
            let sf = if st.o.subs == SubAction::Import {
                match find_companion(&r.path, SUB_EXTS) {
                    Some(f) => Some(f),
                    None => {
                        notes.push(format!("Skipped {}: no subtitle file with the same name", r.path.display()));
                        continue;
                    }
                }
            } else {
                None
            };
            match self.job(&r.path, info, output, af, sf) {
                Ok(j) => jobs.push(j),
                Err(e) => notes.push(format!("Skipped {}: {e}", r.path.display())),
            }
        }
        if jobs.is_empty() {
            return Err(format!("Nothing to do.\n{}", notes.join("\n")));
        }
        let skipped = notes.len();
        notes.insert(0, format!("Prepared {} job(s).", jobs.len()));
        Ok(Plan { jobs, group: Some(("Custom batch".into(), false, skipped)), notes })
    }

    fn wants_folders(&self) -> bool {
        self.st.o.batch
    }
    fn on_drop(&mut self, paths: &[PathBuf], cx: &Ctx) {
        let Some(p) = paths.first() else { return };
        if self.st.o.batch {
            let dir = if p.is_dir() { p.clone() } else { p.parent().map(|d| d.to_path_buf()).unwrap_or_default() };
            self.st.input = s(&dir);
            self.start_scan(cx);
            return;
        }
        for p in paths {
            if has_ext(p, SUB_EXTS) && self.st.o.subs == SubAction::Import {
                self.st.sub_file = s(p);
            } else if has_ext(p, ANY_AUDIO_EXTS) && self.st.o.audio.needs_file() && !self.st.input.trim().is_empty() {
                self.st.audio_file = s(p);
            } else if has_ext(p, &input_exts()) {
                self.st.input = s(p);
                self.st.regen_name();
            }
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }

    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        let o = CustomOptions {
            trim_start: s.trim_start,
            trim_end: s.trim_end,
            video_encode: default_video(s),
            audio_encode: default_audio(s),
            chapter_interval_min: s.chapter_interval_min,
            ..Default::default()
        };
        let base = State { out_dir: s.output_folder.clone(), o, ..Default::default() };
        // partial values (e.g. a factory preset) are laid over the settings-derived defaults
        let v = if v.is_object() { ffenc::presets::overlay(serde_json::to_value(&base).unwrap_or_default(), v) } else { v.clone() };
        self.st = restore(&v, base);
        self.probe = ProbeSlot::default();
        self.rows.clear();
        self.scanned.clear();
        self.scan = None;
    }
}
