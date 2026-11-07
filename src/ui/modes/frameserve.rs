//! Frameserver (AviSynth script): point at a DebugMode FrameServer signpost .avi,
//! get an .avs script that opens it, and encode the script with x264/AAC.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::chapter_creator::{ChapterCreator, Source, Spec};
use crate::ui::common::*;
use ffenc::cmd::chapters::{self, ChapterMode, ManualChapter};
use ffenc::cmd::common::*;
use ffenc::cmd::frameserve::{self as fs, AudioMode, BitDepth, Color, EncodeParams, Matrix, PixelType, ScriptOptions};
use ffenc::ffmpeg::avisynth::{self, Level};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, stem};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Place {
    /// %TEMP%, deleted after the job.
    #[default]
    Temp,
    NextToSignpost,
    Folder,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    signpost: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    suffix: String,
    /// "mp4" or "mkv"
    container: String,
    pixel_type: PixelType,
    audio_mode: AudioMode,
    color: Color,
    matrix: Matrix,
    interlaced: bool,
    trim: bool,
    trim_first: u64,
    trim_last: u64,
    assume_fps: bool,
    fps_num: u32,
    fps_den: u32,
    bits: BitDepth,
    place: Place,
    script_folder: String,
    /// Use `script_text` (typed by the user) instead of the generated script.
    edit_script: bool,
    script_text: String,
    video: VideoEncode,
    audio: AudioEncode,
    resize: bool,
    resize_height: u32,
    chapter_mode: ChapterMode,
    chapter_interval_min: f64,
    manual: Vec<ManualChapter>,
    auto_chapters_if_none: bool,
    faststart: bool,
}

impl State {
    fn from_settings(s: &Settings) -> Self {
        State {
            signpost: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: "_frameserved".into(),
            container: "mp4".into(),
            pixel_type: PixelType::Auto,
            audio_mode: AudioMode::Keep,
            color: Color::Yuv420,
            matrix: Matrix::Rec709,
            interlaced: false,
            trim: false,
            trim_first: 0,
            trim_last: 0,
            assume_fps: false,
            fps_num: 30000,
            fps_den: 1001,
            bits: BitDepth::Eight,
            place: Place::Temp,
            script_folder: String::new(),
            edit_script: false,
            script_text: String::new(),
            video: default_video(s),
            audio: default_audio(s),
            resize: false,
            resize_height: 1080,
            chapter_mode: ChapterMode::Off,
            chapter_interval_min: s.chapter_interval_min,
            manual: vec![],
            auto_chapters_if_none: false,
            faststart: true,
        }
    }
    fn regen_name(&mut self) {
        if self.signpost.trim().is_empty() {
            return;
        }
        let p = PathBuf::from(self.signpost.trim());
        auto_name(&mut self.out_name, &mut self.last_auto, format!("{}{}.{}", stem(&p), self.suffix, self.container));
    }
    fn options(&self) -> ScriptOptions {
        ScriptOptions {
            signpost: PathBuf::from(self.signpost.trim()),
            pixel_type: self.pixel_type,
            audio: self.audio_mode,
            color: self.color,
            matrix: self.matrix,
            interlaced: self.interlaced,
            trim: self.trim.then_some((self.trim_first, self.trim_last)),
            fps: self.assume_fps.then_some((self.fps_num, self.fps_den)),
            bits: self.bits,
        }
    }
    /// The script that will be used: typed or generated.
    fn script(&self) -> Result<String, String> {
        if self.edit_script {
            Ok(self.script_text.clone())
        } else {
            fs::script(&self.options())
        }
    }
    fn script_path(&self) -> Result<(PathBuf, bool), String> {
        let sp = PathBuf::from(self.signpost.trim());
        match self.place {
            Place::Temp => Ok((fs::temp_script_path(), false)),
            Place::NextToSignpost => Ok((fs::saved_script_path(&sp, None), true)),
            Place::Folder => {
                let d = self.script_folder.trim();
                if d.is_empty() {
                    return Err("Choose the folder for the .avs script (or keep it temporary).".into());
                }
                Ok((fs::saved_script_path(&sp, Some(Path::new(d))), true))
            }
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

/// Result of "Test the script": (script text tested, probe result or plain error).
type Tested = (String, Result<MediaInfo, String>);

#[derive(Default)]
pub struct FrameserveMode {
    st: State,
    probe: ProbeSlot,
    creator: ChapterCreator,
    check_for: Option<PathBuf>,
    check: Option<Bg<avisynth::Status>>,
    status: Option<avisynth::Status>,
    test: Option<Bg<Tested>>,
    tested: Option<Tested>,
    show_help: bool,
    show_color_help: bool,
    saved_note: Option<String>,
}

fn ffmpeg_of(ffprobe: &Path) -> PathBuf {
    let ext = ffprobe.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    ffprobe.with_file_name(format!("ffmpeg{ext}"))
}

fn level_color(l: Level) -> egui::Color32 {
    let c = crate::ui::theme::c();
    match l {
        Level::Ok => c.ok,
        Level::Warn => c.warn,
        Level::Error => c.err,
    }
}

/// "1920x1080 · 29.970 fps · 1234 frames · 00:00:41 · audio 48000 Hz 2 ch"
pub fn describe_script(i: &MediaInfo) -> String {
    let mut parts = vec![];
    if let Some(v) = i.video() {
        parts.push(format!("{}x{}", v.width.unwrap_or(0), v.height.unwrap_or(0)));
        if let Some(f) = v.fps {
            parts.push(format!("{f:.3} fps"));
            if let Some(d) = i.any_duration() {
                parts.push(format!("{} frames", (d * f).round() as u64));
            }
        }
        if let Some(p) = &v.pix_fmt {
            parts.push(p.clone());
        }
    }
    if let Some(d) = i.any_duration() {
        parts.push(fmt_hms(d));
    }
    match i.of_type("audio").next() {
        Some(a) => parts.push(format!("audio {} Hz, {} ch", a.sample_rate.unwrap_or(0), a.channels.unwrap_or(0))),
        None => parts.push("no audio".into()),
    }
    parts.join(" · ")
}

impl FrameserveMode {
    /// Length of the output: tested script, else the signpost minus the trim.
    fn duration(&self) -> Option<f64> {
        let st = &self.st;
        if let (Some((text, Ok(i))), Ok(cur)) = (&self.tested, st.script()) {
            if *text == cur {
                return i.any_duration();
            }
        }
        let info = self.probe.info()?;
        let full = info.any_duration()?;
        let fps = if st.assume_fps && st.fps_den > 0 { st.fps_num as f64 / st.fps_den as f64 } else { info.video().and_then(|v| v.fps).unwrap_or(0.0) };
        if st.trim && fps > 0.0 {
            let total = (full * fps).round();
            let last = if st.trim_last == 0 { total - 1.0 } else { (st.trim_last as f64).min(total - 1.0) };
            return Some(((last - st.trim_first as f64 + 1.0) / fps).max(0.0));
        }
        if st.assume_fps && fps > 0.0 {
            let orig = info.video().and_then(|v| v.fps).unwrap_or(fps);
            return Some(full * orig / fps);
        }
        Some(full)
    }

    fn start_check(&mut self, cx: &Ctx) {
        let Some(fp) = cx.ffprobe else { return };
        let ff = ffmpeg_of(fp);
        if self.check_for.as_ref() == Some(&ff) {
            return;
        }
        self.check_for = Some(ff.clone());
        self.status = None;
        self.check = Some(Bg::spawn(cx.egui, move || avisynth::check(&ff)));
    }

    fn start_test(&mut self, cx: &Ctx) {
        let text = match self.st.script() {
            Ok(t) => t,
            Err(e) => {
                self.tested = Some((String::new(), Err(e)));
                return;
            }
        };
        let Some(fp) = cx.ffprobe.map(|p| p.to_path_buf()) else {
            self.tested = Some((text, Err("FFprobe not found – set the FFmpeg folder in Settings.".into())));
            return;
        };
        let keep = cx.settings.keep_temp_files;
        self.tested = None;
        self.test = Some(Bg::spawn(cx.egui, move || {
            let path = std::env::temp_dir().join(format!("{}test_{}.avs", fs::TEMP_PREFIX, ffenc::util::unique_id()));
            if let Err(e) = std::fs::write(&path, &text) {
                return (text, Err(format!("Could not write the test script {}: {e}", path.display())));
            }
            let r = probe::probe(&fp, &path).map_err(|e| fs::explain_error(&e));
            if !keep {
                let _ = std::fs::remove_file(&path);
            }
            (text, r)
        }));
    }

    fn save_now(&mut self) {
        let st = &self.st;
        let r = (|| -> Result<String, String> {
            let text = st.script()?;
            let sp = PathBuf::from(st.signpost.trim());
            let path = match st.place {
                Place::Folder if !st.script_folder.trim().is_empty() => fs::saved_script_path(&sp, Some(Path::new(st.script_folder.trim()))),
                Place::Folder => return Err("Choose the folder first.".into()),
                _ => fs::saved_script_path(&sp, None),
            };
            std::fs::write(&path, text).map_err(|e| format!("Could not save {}: {e}", path.display()))?;
            Ok(format!("Saved {}", path.display()))
        })();
        self.saved_note = Some(r.unwrap_or_else(|e| e));
    }
}

const HELP: &str = "How it works: your editor (Premiere, Vegas, After Effects…) serves the finished timeline through DebugMode FrameServer, which writes a small \"signpost\" .avi file. Pick that file here. This preset writes an AviSynth script (.avs) that opens the signpost with AviSource, and FFmpeg encodes the script to MP4 or MKV.\n\n\
Before you run: 1) keep the project open and serving in the editor until the encode finishes – the signpost only works while it is serving; 2) AviSynth+ must be installed, and it must match FFmpeg: this 64-bit FFmpeg needs 64-bit AviSynth+ (the AviSynth+ installer puts both in Windows); 3) use \"Test the script\" to check that everything connects.\n\n\
If AviSource can't open the signpost, other AviSynth source filters such as DirectShowSource or FFVideoSource (FFMS2 plugin) can sometimes read it – type them in the script editor below. They are not needed normally.";

impl Mode for FrameserveMode {
    fn key(&self) -> &'static str {
        "frameserve"
    }
    fn title(&self) -> &'static str {
        "Frameserver (AviSynth script)"
    }
    fn description(&self) -> &'static str {
        "Encode straight from your editor through DebugMode FrameServer: pick the signpost .avi, the app writes the AviSynth script and encodes it with x264 + AAC."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["signpost", "out_name", "last_auto", "script_text", "edit_script"]
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        self.start_check(cx);
        if let Some(r) = self.check.as_ref().and_then(|c| c.poll()) {
            self.status = Some(r);
            self.check = None;
        }
        if let Some(r) = self.test.as_ref().and_then(|t| t.poll()) {
            self.tested = Some(r);
            self.test = None;
        }
        if self.probe.update(cx.egui, cx.ffprobe, &self.st.signpost) {
            // prefill the frame rate from the signpost
            if let Some(f) = self.probe.info().and_then(|i| i.video()).and_then(|v| v.fps) {
                if !self.st.assume_fps {
                    if let Some(c) = fs::FPS_CHOICES.iter().min_by(|a, b| ((a.1 as f64 / a.2 as f64) - f).abs().total_cmp(&((b.1 as f64 / b.2 as f64) - f).abs())) {
                        self.st.fps_num = c.1;
                        self.st.fps_den = c.2;
                    }
                }
            }
        }

        // help + environment check
        ui.horizontal_wrapped(|ui| {
            let r = ui.small_button(if self.show_help { "Hide help" } else { "How frameserving works…" });
            if named(r, egui::WidgetType::Button, if self.show_help { "Hide the frameserving help" } else { "Show help: how frameserving works and what you need" }).clicked() {
                self.show_help = !self.show_help;
            }
        });
        if self.show_help {
            ui.label(HELP);
        }
        match &self.status {
            None if self.check.is_some() => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking AviSynth support…");
                });
            }
            None => hint(ui, "Set the FFmpeg folder to check AviSynth support."),
            Some(s) => {
                for (l, t) in avisynth::advice(s) {
                    let mark = match l {
                        Level::Ok => "OK:",
                        Level::Warn => "Note:",
                        Level::Error => "Problem:",
                    };
                    ui.label(RichText::new(format!("{mark} {t}")).color(level_color(l)));
                }
            }
        }
        ui.separator();

        let st = &mut self.st;
        if st.out_name.trim().is_empty() && !st.signpost.trim().is_empty() {
            st.regen_name();
        }
        egui::Grid::new("fs_in").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Signpost .avi", &mut st.signpost, Pick::File("Signpost AVI", &["avi"]), &cx.settings.input_folder) {
                st.regen_name();
                self.tested = None;
            }
            ui.label("");
            ui.vertical(|ui| {
                self.probe.status_ui(ui, |i| describe_script(i));
                if self.probe.result.as_ref().map(|r| r.is_err()).unwrap_or(false) {
                    hint(ui, "ffprobe can't always read a signpost directly – that's fine; use “Test the script” below.");
                }
            });
            ui.end_row();
        });

        section(ui, "AviSynth script");
        egui::Grid::new("fs_script").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let l = crate::ui::common::row_label(ui, "Colour format");
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    let r = egui::ComboBox::from_id_salt("fs_color").width(330.0).selected_text(st.color.label()).show_ui(ui, |ui| {
                        for c in Color::ALL {
                            ui.selectable_value(&mut st.color, c, c.label()).on_hover_text(c.help());
                        }
                    });
                    r.response.labelled_by(l.id);
                    let h = ui.small_button(if self.show_color_help { "Hide help" } else { "Which one? (help)" });
                    if named(h, egui::WidgetType::Button, if self.show_color_help { "Hide the colour format help" } else { "Help: explain each colour format and which to choose" }).clicked() {
                        self.show_color_help = !self.show_color_help;
                    }
                });
                hint(ui, st.color.help());
                if self.show_color_help {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new("Which colour format?").strong());
                        for c in Color::ALL {
                            ui.label(format!("• {}: {}", c.label(), c.help()));
                        }
                        ui.label("If unsure, keep YUV420. ConvertToYUV420, YUV422 and YUV444 need AviSynth+ (the current AviSynth); with the old AviSynth 2.6 use YV12.");
                    });
                }
            });
            ui.end_row();

            let l = crate::ui::common::row_label(ui, "Colour matrix");
            ui.vertical(|ui| {
                let enabled = st.color != Color::None;
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt("fs_matrix").width(330.0).selected_text(st.matrix.label()).show_ui(ui, |ui| {
                        for m in Matrix::ALL {
                            ui.selectable_value(&mut st.matrix, m, m.label());
                        }
                    }).response.labelled_by(l.id);
                    ui.checkbox(&mut st.interlaced, "Interlaced source (interlaced=true)");
                });
                hint(ui, "Used when converting between RGB and YUV. Rec709 suits HD projects.");
            });
            ui.end_row();

            crate::ui::common::row_label(ui, "Bit depth");
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut st.bits, BitDepth::Eight, "8-bit (plays everywhere)");
                let ten_ok = !matches!(st.color, Color::Yv12 | Color::Rgb32);
                ui.add_enabled_ui(ten_ok, |ui| ui.radio_value(&mut st.bits, BitDepth::Ten, "10-bit (ConvertBits(10); smoother gradients, less compatible)"));
                if !ten_ok && st.bits == BitDepth::Ten {
                    st.bits = BitDepth::Eight;
                }
            });
            ui.end_row();

            let l = crate::ui::common::row_label(ui, "Signpost format");
            ui.vertical(|ui| {
                egui::ComboBox::from_id_salt("fs_pt").width(220.0).selected_text(st.pixel_type.label()).show_ui(ui, |ui| {
                    for p in PixelType::ALL {
                        ui.selectable_value(&mut st.pixel_type, p, p.label());
                    }
                }).response.labelled_by(l.id);
                hint(ui, "AviSource pixel_type. Leave on Automatic unless the picture looks wrong (skewed or odd colours).");
            });
            ui.end_row();

            let l = crate::ui::common::row_label(ui, "Audio");
            egui::ComboBox::from_id_salt("fs_audio").width(330.0).selected_text(st.audio_mode.label()).show_ui(ui, |ui| {
                for a in AudioMode::ALL {
                    ui.selectable_value(&mut st.audio_mode, a, a.label());
                }
            }).response.labelled_by(l.id);
            ui.end_row();

            crate::ui::common::row_label_tip(ui, "Frame rate", "Normally the frame rate of the signpost file is used. Force one only if the editor exported the wrong rate.");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut st.assume_fps, "Force a frame rate (AssumeFPS)");
                if st.assume_fps {
                    let cur = fs::FPS_CHOICES.iter().find(|c| c.1 == st.fps_num && c.2 == st.fps_den).map(|c| c.0.to_string()).unwrap_or_else(|| format!("{}/{}", st.fps_num, st.fps_den));
                    egui::ComboBox::from_id_salt("fs_fps").selected_text(format!("{cur} fps")).show_ui(ui, |ui| {
                        for c in fs::FPS_CHOICES {
                            if ui.selectable_label(st.fps_num == c.1 && st.fps_den == c.2, format!("{} fps", c.0)).clicked() {
                                st.fps_num = c.1;
                                st.fps_den = c.2;
                            }
                        }
                    }).response.on_hover_text("Frame rate");
                }
            });
            ui.end_row();

            crate::ui::common::row_label(ui, "Trim");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut st.trim, "Only part of the timeline (frames)");
                if st.trim {
                    let l1 = ui.label("first frame");
                    ui.add(egui::DragValue::new(&mut st.trim_first).range(0..=u64::MAX)).labelled_by(l1.id);
                    let l2 = ui.label("last frame (0 = to the end)");
                    ui.add(egui::DragValue::new(&mut st.trim_last).range(0..=u64::MAX)).labelled_by(l2.id);
                }
            });
            ui.end_row();

            crate::ui::common::row_label(ui, "Keep the .avs file");
            ui.vertical(|ui| {
                ui.radio_value(&mut st.place, Place::Temp, "No – temporary file in %TEMP%, deleted after the job");
                ui.radio_value(&mut st.place, Place::NextToSignpost, "Save it next to the signpost");
                ui.radio_value(&mut st.place, Place::Folder, "Save it in a folder I choose");
                if st.place == Place::Folder {
                    ui.horizontal(|ui| {
                        path_row_nodrop(ui, "Folder", &mut st.script_folder, Pick::Folder, &cx.settings.output_folder);
                    });
                }
                if st.place != Place::Temp {
                    if let Ok((p, _)) = st.script_path() {
                        hint(ui, format!("Saved as {}{}", p.display(), if p.exists() { " (replaces the existing file of that name)" } else { "" }));
                    }
                }
            });
            ui.end_row();
        });

        // preview / editor
        ui.add_space(4.0);
        let was_editing = st.edit_script;
        ui.checkbox(&mut st.edit_script, "Edit the script by hand (for power users)");
        if st.edit_script && !was_editing && st.script_text.trim().is_empty() {
            st.script_text = fs::script(&st.options()).unwrap_or_default();
        }
        let generated = fs::script(&st.options());
        let script_label;
        if st.edit_script {
            script_label = ui.label("Script text (edited – the options above no longer change it):");
            let r = ui.add(egui::TextEdit::multiline(&mut st.script_text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(6));
            r.labelled_by(script_label.id);
            if ui.small_button("Reset to the generated script").clicked() {
                st.script_text = generated.clone().unwrap_or_default();
            }
        } else {
            script_label = ui.label("Script preview:");
            match &generated {
                Ok(t) => {
                    let r = ui.add(egui::TextEdit::multiline(&mut t.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(4));
                    r.labelled_by(script_label.id);
                }
                Err(e) => {
                    ui.colored_label(crate::ui::theme::c().err, e);
                }
            }
        }
        let mut test = false;
        let mut save = false;
        ui.horizontal_wrapped(|ui| {
            if copy_button(ui, "Copy script", "Copy the AviSynth script text").clicked() {
                if let Ok(t) = st.script() {
                    ui.ctx().copy_text(t);
                }
            }
            let busy = self.test.is_some();
            test = ui.add_enabled(!busy && !st.signpost.trim().is_empty(), egui::Button::new("Test the script")).on_hover_text("Open the script with ffprobe and show what it delivers").clicked();
            save = ui.add_enabled(!st.signpost.trim().is_empty(), egui::Button::new("Save the .avs now")).on_hover_text("Write the script next to the signpost (or to the chosen folder) without encoding").clicked();
            if busy {
                ui.spinner();
                ui.label("Testing…");
            }
        });
        if let Some(n) = &self.saved_note {
            hint(ui, n.clone());
        }
        if let Some((text, r)) = &self.tested {
            let stale = st.script().map(|t| t != *text).unwrap_or(true);
            match r {
                Ok(i) => {
                    ui.label(RichText::new(format!("Test OK: {}{}", describe_script(i), if stale { " (script changed since)" } else { "" })).color(crate::ui::theme::c().ok));
                }
                Err(e) => {
                    ui.label(RichText::new(format!("Test failed: {e}")).color(crate::ui::theme::c().err));
                }
            }
        }

        section(ui, "Video (libx264)");
        egui::Grid::new("fs_v").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            video_encode_ui(ui, "fs", &mut st.video, true);
            crate::ui::common::row_label(ui, "Resize");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut st.resize, "New vertical resolution");
                if st.resize {
                    num_u32(ui, &mut st.resize_height, 1..=4320, " px");
                    for h in [720, 1080, 1440, 2160] {
                        if ui.small_button(h.to_string()).clicked() {
                            st.resize_height = h;
                        }
                    }
                }
            });
            ui.end_row();
            crate::ui::common::row_label(ui, "Encoder format");
            hint(ui, format!("{} (follows the colour format and bit depth above)", st.color.pix_fmt(st.bits)));
            ui.end_row();
        });
        if st.audio_mode != AudioMode::None {
            section(ui, "Audio (AAC)");
            egui::Grid::new("fs_a").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| audio_encode_ui(ui, "fsa", &mut st.audio));
        }

        section(ui, "Chapters & output");
        let dur = self.duration();
        let st = &mut self.st;
        let src = Source { info: self.probe.info(), probing: self.probe.busy(), out_dur: dur, trim_start: 0.0, batch_note: None };
        let creator = &mut self.creator;
        egui::Grid::new("fs_o").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) {
                st.regen_name();
            }
            let l = crate::ui::common::row_label(ui, "Container");
            let before = st.container.clone();
            egui::ComboBox::from_id_salt("fs_cont").selected_text(st.container.clone()).show_ui(ui, |ui| {
                ui.selectable_value(&mut st.container, "mp4".to_string(), "mp4 (plays everywhere)");
                ui.selectable_value(&mut st.container, "mkv".to_string(), "mkv");
            }).response.labelled_by(l.id);
            if before != st.container {
                let n = PathBuf::from(&st.out_name);
                if !st.out_name.trim().is_empty() {
                    st.out_name = format!("{}.{}", stem(&n), st.container);
                }
                st.regen_name();
            }
            ui.end_row();
            let mut spec = Spec { mode: &mut st.chapter_mode, interval_min: &mut st.chapter_interval_min, manual: &mut st.manual, auto_if_none: &mut st.auto_chapters_if_none };
            creator.grid_rows(ui, "fs", &mut spec, &src);
            faststart_ui(ui, &mut st.faststart, st.container == "mp4");
        });
        if test {
            self.start_test(cx);
        }
        if save {
            self.save_now();
        }
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        ffenc::cmd::common::parse_output_fps(&self.st.video.output_fps)?;
        if let Some(s) = &self.status {
            if s.demuxer == Some(false) {
                return Err(avisynth::advice(s).into_iter().find(|a| a.0 == Level::Error).map(|a| a.1).unwrap_or_default());
            }
        }
        let st = &self.st;
        let signpost = check_input(&st.signpost, &["avi"], "signpost .avi")?;
        let text = st.script()?;
        let (script_path, keep) = st.script_path()?;
        let auto = format!("{}{}.{}", stem(&signpost), st.suffix, st.container);
        let output = check_output(&st.out_dir, if st.out_name.trim().is_empty() { &auto } else { &st.out_name })?;
        if !matches!(ffenc::util::ext_lower(&output).as_str(), "mp4" | "mkv" | "m4v" | "mov") {
            return Err("The output file must end in .mp4 or .mkv.".into());
        }
        if same_file(&output, &signpost) || same_file(&output, &script_path) {
            return Err("The output must be a different file from the signpost and the script.".into());
        }
        let dur = self.duration();
        let resize = if st.resize {
            let (w, h) = match (&self.tested, self.probe.info()) {
                (Some((_, Ok(i))), _) | (_, Some(i)) => i.video().map(|v| (v.width.unwrap_or(0), v.height.unwrap_or(0))).unwrap_or((0, 0)),
                _ => (0, 0),
            };
            if w == 0 || h == 0 {
                return Err("Resize needs the picture size: use “Test the script” first.".into());
            }
            Some(resize_dims(w, h, st.resize_height)?)
        } else {
            None
        };
        let mode = if st.chapter_mode == ChapterMode::Off && st.auto_chapters_if_none { ChapterMode::Auto } else { st.chapter_mode };
        let chapters = if mode == ChapterMode::Off {
            None
        } else {
            let d = dur.ok_or("Chapters need the length: use “Test the script” first.")?;
            chapters::chapter_input(d, mode, st.chapter_interval_min * 60.0, &st.manual)?
        };
        let pr = EncodeParams {
            script_path: script_path.clone(),
            script_text: text.clone(),
            output: output.clone(),
            video: st.video.clone(),
            audio: st.audio.clone(),
            has_audio: st.audio_mode != AudioMode::None,
            pix_fmt: st.color.pix_fmt(st.bits),
            resize,
            bt709_tags: st.color != Color::None && st.color != Color::Rgb32 && st.matrix == Matrix::Rec709,
            chapters,
            faststart: st.faststart,
        };
        let mut cmd = fs::build(&pr)?;
        if st.edit_script {
            cmd.warnings.push("Using your hand-edited script.".into());
        }
        if let Some(s) = &self.status {
            for (l, t) in avisynth::advice(s) {
                if l == Level::Error {
                    cmd.warnings.push(t);
                }
            }
        }
        let tested_ok = matches!(&self.tested, Some((t, Ok(_))) if *t == text);
        let mut notes = vec!["Keep the project open and serving in your editor until the encode has finished.".to_string()];
        if !tested_ok {
            notes.push("Tip: “Test the script” checks the connection before you start.".into());
        }
        if keep {
            notes.push(format!("The script is saved as {} and kept.", script_path.display()));
        }
        let _ = cx;
        let mut job = JobSpec::new(format!("Frameserve {}", signpost.file_name().unwrap_or_default().to_string_lossy()), cmd, output.clone(), log_path_for(&output), dur);
        job.keep_side_files = keep;
        Ok(Plan { jobs: vec![job], notes, ..Default::default() })
    }

    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(p) = first_with_ext(paths, &["avi"]) {
            self.st.signpost = s(p);
            self.st.regen_name();
            self.tested = None;
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }

    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        self.probe = ProbeSlot::default();
        self.tested = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_yuv420_and_paths() {
        let m = FrameserveMode::default();
        assert_eq!(m.st.color, Color::Yuv420, "default colour conversion is ConvertToYUV420()");
        assert_eq!(m.st.place, Place::Temp);
        let mut st = State { signpost: r"/v/My Clip.avi".into(), ..State::default() };
        st.regen_name();
        assert_eq!(st.out_name, "My Clip_frameserved.mp4");
        let t = st.script().unwrap();
        assert!(t.contains("ConvertToYUV420(matrix=\"Rec709\")"), "{t}");
        // temp script: fresh temp path, not kept
        let (p, keep) = st.script_path().unwrap();
        assert!(!keep && p.starts_with(std::env::temp_dir()));
        st.place = Place::NextToSignpost;
        assert_eq!(st.script_path().unwrap(), (PathBuf::from("/v/My Clip_ffenc.avs"), true));
        st.place = Place::Folder;
        assert!(st.script_path().is_err());
        st.script_folder = "/s".into();
        assert_eq!(st.script_path().unwrap().0, PathBuf::from("/s/My Clip_ffenc.avs"));
        // hand-edited text wins
        st.edit_script = true;
        st.script_text = "AviSource(\"x.avi\")\r\n".into();
        assert_eq!(st.script().unwrap(), "AviSource(\"x.avi\")\r\n");
        assert_eq!(ffmpeg_of(Path::new("/f/ffprobe.exe")), PathBuf::from("/f/ffmpeg.exe"));
        assert_eq!(ffmpeg_of(Path::new("/f/ffprobe")), PathBuf::from("/f/ffmpeg"));
    }
}
