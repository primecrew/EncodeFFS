//! Frameserver (AviSynth script): point at a DebugMode FrameServer signpost .avi,
//! get an .avs script that opens it, and encode the script with H.264/AAC by default
//! (HEVC / AV1 and AC3 / copy are offered, same pickers as Re-encode).

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::chapter_creator::{ChapterCreator, Source, Spec};
use crate::ui::common::*;
use ffenc::cmd::chapters::{self, ChapterMode, ManualChapter};
use ffenc::cmd::common::*;
use ffenc::cmd::conflicts::{self, ConflictKind, Choices as ConflictChoices};
use ffenc::cmd::frameserve::{self as fs, AudioMode, BitDepth, Color, EncodeParams, Matrix, PixelType, ScriptOptions};
use ffenc::ffmpeg::avisynth::{self, Level};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use crate::ui::option_tips;
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
    /// When omitted from an old saved preset, loads as Rec709 (the previous default).
    #[serde(default = "ffenc::cmd::frameserve::missing_matrix")]
    matrix: Matrix,
    interlaced: bool,
    trim: bool,
    trim_first: u64,
    trim_last: u64,
    bits: BitDepth,
    /// Only used with PC.709 / PC.601: squeeze full range to TV range in FFmpeg.
    /// Rec709 / Rec601 / Rec2020 already squeeze in the script; this stays off.
    #[serde(default)]
    range_convert: bool,
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
    #[serde(default = "ffenc::cmd::common::missing_faststart")]
    faststart: bool,
    /// Crop / watermark (1.1).
    picture: ffenc::cmd::picture::PictureSettings,
    /// Session-only: “Keep my setting” hides a warning until the combo changes.
    #[serde(skip)]
    dismissed: Vec<ConflictKind>,
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
            matrix: Matrix::Pc709,
            interlaced: false,
            trim: false,
            trim_first: 0,
            trim_last: 0,
            bits: BitDepth::Eight,
            range_convert: false,
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
            faststart: false,
            picture: Default::default(),
            dismissed: vec![],
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
            bits: conflicts::script_bit_depth(&self.video),
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

    fn conflict_choices(&self) -> ConflictChoices {
        let yuv = !matches!(self.color, Color::None | Color::Rgb32);
        let script_text = if self.edit_script { self.script_text.clone() } else { String::new() };
        ConflictChoices {
            color: self.color,
            matrix: self.matrix,
            bits: conflicts::effective_bits(self.edit_script, &script_text, &self.video),
            range_convert: self.range_convert,
            bt709_tags: yuv && self.matrix == Matrix::Rec709,
            video: self.video.clone(),
            audio: self.audio.clone(),
            container: self.container.clone(),
            out_name: self.out_name.clone(),
            last_auto: self.last_auto.clone(),
            output: PathBuf::from(self.out_dir.trim()).join(self.out_name.trim()),
            edit_script: self.edit_script,
            script_text,
            uses_script_matrix: true,
            source_range: None,
        }
    }

    fn apply_conflict(&mut self, kind: ConflictKind) {
        let mut c = self.conflict_choices();
        conflicts::apply(&mut c, kind);
        self.color = c.color;
        self.matrix = c.matrix;
        self.range_convert = c.range_convert;
        self.video = c.video;
        self.audio = c.audio;
        if self.container != c.container {
            self.container = c.container;
            self.out_name = c.out_name;
            self.last_auto = c.last_auto;
        }
        if self.edit_script {
            self.script_text = c.script_text;
        }
        self.bits = conflicts::script_bit_depth(&self.video);
    }

    /// After loading a saved preset: map old script Bit depth onto the codec 10-bit
    /// checkbox when the codec supports it. H.264 stays 8-bit.
    /// Old presets (before 1.1) could force a frame rate in the script ("assume_fps" with
    /// "fps_num"/"fps_den"). That option is gone; the rate moves to Output frame rate when that box
    /// is empty, so the preset still converts to the rate the user chose. Otherwise it is ignored.
    fn migrate_forced_fps(&mut self, v: &serde_json::Value) {
        if v.get("assume_fps").and_then(|x| x.as_bool()) != Some(true) || !self.video.output_fps.trim().is_empty() {
            return;
        }
        let num = v.get("fps_num").and_then(|x| x.as_u64()).unwrap_or(0);
        let den = v.get("fps_den").and_then(|x| x.as_u64()).unwrap_or(0);
        if num == 0 || den == 0 {
            return;
        }
        let rate = if den == 1 { num.to_string() } else { format!("{num}/{den}") };
        if ffenc::cmd::common::parse_output_fps(&rate).ok().flatten().is_some() {
            self.video.output_fps = rate;
        }
    }

    fn migrate_bit_depth(&mut self) {
        if self.video.codec != ffenc::cmd::codecs::VideoCodec::H264 {
            self.video.ten_bit = self.bits == BitDepth::Ten;
        }
        self.bits = conflicts::script_bit_depth(&self.video);
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
    pic: crate::ui::picture::PictureRt,
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
        // trim frame numbers are source frames
        let fps = info.video().and_then(|v| v.fps).unwrap_or(0.0);
        if st.trim && fps > 0.0 {
            let total = (full * fps).round();
            let last = if st.trim_last == 0 { total - 1.0 } else { (st.trim_last as f64).min(total - 1.0) };
            return Some(((last - st.trim_first as f64 + 1.0) / fps).max(0.0));
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


/// Draw matching warnings (or all, when `filter` is empty) with Fix / Keep buttons.
fn show_conflicts(ui: &mut egui::Ui, st: &mut State, filter: &[ConflictKind]) {
    let current = conflicts::detect(&st.conflict_choices());
    conflicts::prune_dismissed(&mut st.dismissed, &current);
    for c in current {
        if !filter.is_empty() && !filter.contains(&c.kind) {
            continue;
        }
        if st.dismissed.contains(&c.kind) {
            continue;
        }
        match conflict_warning_ui(ui, &c, true) {
            ConflictAction::Fix(k) => st.apply_conflict(k),
            ConflictAction::Keep(k) => st.dismissed.push(k),
            ConflictAction::None => {}
        }
    }
}

const INSTALL_HELP: &str = "Opens the user guide at “Installing the frameserving tools”: where to download AviSynth+ and DebugMode FrameServer, and how to install them.";

const HELP: &str = "How it works: your editor (Premiere, Vegas, After Effects…) serves the finished timeline through DebugMode FrameServer, which writes a small \"signpost\" .avi file. Pick that file here. This preset writes an AviSynth script (.avs) that opens the signpost with AviSource, and FFmpeg encodes the script to MP4 or MKV.\n\n\
Before you run: 1) keep the project open and serving in the editor until the encode finishes – the signpost only works while it is serving; 2) AviSynth+ must be installed, and it must match FFmpeg: this 64-bit FFmpeg needs 64-bit AviSynth+ (the AviSynth+ installer puts both in Windows); 3) use \"Test the script\" to check that everything connects.\n\n\
If AviSource can't open the signpost, other AviSynth source filters such as DirectShowSource or FFVideoSource (FFMS2 plugin) can sometimes read it – type them in the script editor below. They are not needed normally.

Video and audio codecs: the default is H.264 (libx264) and AAC, the same as before. You can also pick HEVC or AV1, and AC3 or keep the original audio, just like Re-encode. MP4 and MKV both work for these.

Colour range: the default colour matrix is PC.709, which keeps full range (recommended for Vegas 8-bit projects). Rec709, Rec601 and Rec2020 already convert to TV (limited) range in the script; use Rec709 for 32-bit full-range Vegas projects. The default is not to apply a second full-to-TV conversion (that would look washed out). You can still tick “Convert full range to TV range” if you want to; a warning is shown and the encode still runs. PC.709 and PC.601 keep the full range from the editor; the file is tagged as full range, not TV, unless you tick that box.

10-bit: for HEVC and AV1, the 10-bit box in the codec section both writes a 10-bit pixel format and adds ConvertBits(10) at the end of the script (after the colour conversion), so FFmpeg does not convert back to 8-bit. H.264 is always 8-bit. Unusual combinations (for example 10-bit with YV12) show a warning and still run. If you edit the script by hand and its bit depth does not match the encoder, a warning offers a one-click fix.";

impl Mode for FrameserveMode {
    fn key(&self) -> &'static str {
        "frameserve"
    }
    fn title(&self) -> &'static str {
        "Frameserver (AviSynth script)"
    }
    fn description(&self) -> &'static str {
        "Encode straight from your editor through DebugMode FrameServer: pick the signpost .avi, the app writes the AviSynth script and encodes it with H.264 + AAC by default (or HEVC / AV1 and AC3 / copy)."
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
        self.probe.update(cx.egui, cx.ffprobe, &self.st.signpost);

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
            None => hint(ui, "Choose an FFmpeg (Settings › FFmpeg to use) to check AviSynth support."),
            Some(s) => {
                let advice = avisynth::advice(s);
                for (l, t) in &advice {
                    let mark = match l {
                        Level::Ok => "OK:",
                        Level::Warn => "Note:",
                        Level::Error => "Problem:",
                    };
                    ui.label(RichText::new(format!("{mark} {t}")).color(level_color(*l)));
                }
                if advice.iter().any(|(l, _)| *l == Level::Error) {
                    if ui.small_button("How to install…").on_hover_text(INSTALL_HELP).clicked() {
                        crate::ui::open_guide_page("frameserver", "installing-the-frameserving-tools");
                    }
                }
            }
        }
        ui.separator();

        // crop / watermark previews: the signpost (a picture appears once the editor is serving)
        let pic_srcs: Vec<crate::ui::picture::PicSource> = self
            .tested
            .as_ref()
            .and_then(|(_, r)| r.as_ref().ok())
            .or(self.probe.info())
            .and_then(|i| crate::ui::picture::PicSource::from_info(Path::new(self.st.signpost.trim()), i))
            .map(|mut p| {
                p.duration = self.duration().unwrap_or(p.duration);
                p
            })
            .into_iter()
            .collect();
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
            let l = crate::ui::common::row_label_tip(ui, "Colour format", option_tips::COLOUR_FORMAT);
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

            let l = crate::ui::common::row_label_tip(ui, "Colour matrix", option_tips::COLOUR_MATRIX);
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
                hint(ui, "Used when converting between RGB and YUV. PC.709 is the usual choice for Vegas 8-bit projects; Rec709 squeezes to TV range.");
                if matches!(st.color, Color::None | Color::Rgb32) {
                    hint(ui, "No YUV conversion in the script, so the matrix is not applied.");
                } else {
                    if st.matrix.squeezes_to_tv() || st.matrix == Matrix::Default {
                        hint(ui, "This matrix already converts to TV (limited) range in the script. Leave the box below unticked unless you really want a second conversion.");
                    } else if st.matrix.keeps_full_range() {
                        hint(ui, "PC.709 / PC.601 keep the full (0–255) range from the editor. The file is tagged as full range, not TV, unless you tick the box below.");
                    }
                    ui.checkbox(&mut st.range_convert, "Convert full range to TV range");
                    if st.range_convert && !(st.matrix.squeezes_to_tv() || st.matrix == Matrix::Default) {
                        hint(ui, "FFmpeg will squeeze full range to TV (16–235) once, then tag the file as TV range.");
                    }
                    show_conflicts(ui, st, &[ConflictKind::DoubleRangeSqueeze]);
                }
            });
            ui.end_row();

            let l = crate::ui::common::row_label_tip(ui, "Signpost format", option_tips::SIGNPOST_FORMAT);
            ui.vertical(|ui| {
                egui::ComboBox::from_id_salt("fs_pt").width(220.0).selected_text(st.pixel_type.label()).show_ui(ui, |ui| {
                    for p in PixelType::ALL {
                        ui.selectable_value(&mut st.pixel_type, p, p.label());
                    }
                }).response.labelled_by(l.id);
                hint(ui, "AviSource pixel_type. Leave on Automatic unless the picture looks wrong (skewed or odd colours).");
            });
            ui.end_row();

            let l = crate::ui::common::row_label_tip(ui, "Audio", option_tips::FRAMESERVE_AUDIO);
            egui::ComboBox::from_id_salt("fs_audio").width(330.0).selected_text(st.audio_mode.label()).show_ui(ui, |ui| {
                for a in AudioMode::ALL {
                    ui.selectable_value(&mut st.audio_mode, a, a.label());
                }
            }).response.labelled_by(l.id);
            ui.end_row();

            crate::ui::common::row_label_tip(ui, "Trim", option_tips::TRIM_FRAMES);
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

        section(ui, &format!("Video ({})", st.video.codec.encoder()));
        egui::Grid::new("fs_v").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::codecs_ui::video_codec_ui_ex(ui, "fs", &mut st.video, None, Some("Also sets the AviSynth script to 10-bit."), true, &cx.ff_status());
            st.bits = conflicts::script_bit_depth(&st.video);
            ui.label("");
            show_conflicts(ui, st, &[
                ConflictKind::TenBitEightBitColor,
                ConflictKind::EncoderTenBitScriptEight,
                ConflictKind::ScriptTenBitEncoderEight,
                ConflictKind::ScriptTenBitH264,
            ]);
            ui.end_row();
            crate::ui::common::row_label_tip(ui, "Resize", option_tips::RESIZE);
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
            crate::ui::common::row_label_tip(ui, "Encoder format", "The pixel format written by the encoder. It follows the colour format and the 10-bit option (not only 4:2:0), so a 10-bit script is not forced back to 8-bit.");
            hint(ui, format!("{} (follows the colour format and the 10-bit option)", st.color.pix_fmt(conflicts::script_bit_depth(&st.video))));
            ui.end_row();
            crate::ui::picture::watermark_row(ui, &mut st.picture.watermark, &mut self.pic);
        });
        crate::ui::picture::crop_section(ui, cx, "fs", &mut st.picture.crop, &pic_srcs, &mut self.pic);
        crate::ui::picture::watermark_window(cx, "fs", &mut st.picture, &pic_srcs, &mut self.pic);
        if st.picture.any() && pic_srcs.is_empty() {
            hint(ui, "Crop and watermark need the picture size: use “Test the script”.");
        }
        if st.audio_mode != AudioMode::None {
            section(ui, "Audio");
            egui::Grid::new("fs_a").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                crate::ui::codecs_ui::audio_codec_ui(ui, "fsa", &mut st.audio, true, &cx.ff_status());
                ui.label("");
                show_conflicts(ui, st, &[ConflictKind::CopyIgnoresFilters]);
                ui.end_row();
            });
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
            let l = crate::ui::common::row_label_tip(ui, "Container", option_tips::CONTAINER);
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
        show_conflicts(ui, st, &[ConflictKind::Av1InMov, ConflictKind::CopyAudioIntoMp4]);
        if test {
            self.start_test(cx);
        }
        if save {
            self.save_now();
        }
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        ffenc::cmd::common::parse_output_fps(&self.st.video.output_fps)?;
        crate::ui::codecs_ui::check_available(
            cx,
            Some(&self.st.video),
            (self.st.audio_mode != AudioMode::None).then_some(&self.st.audio),
        )?;
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
        let (w, h) = match (&self.tested, self.probe.info()) {
            (Some((_, Ok(i))), _) | (_, Some(i)) => i.video().map(|v| (v.width.unwrap_or(0), v.height.unwrap_or(0))).unwrap_or((0, 0)),
            _ => (0, 0),
        };
        if st.picture.any() && (w == 0 || h == 0) {
            return Err("Crop / watermark need the picture size: use “Test the script” first.".into());
        }
        let (picture, pic_warnings) = crate::ui::picture::resolve_for(&st.picture, &self.pic, w, h)?;
        let resize = if st.resize {
            if w == 0 || h == 0 {
                return Err("Resize needs the picture size: use “Test the script” first.".into());
            }
            let (cw, ch) = picture.cropped_size(w, h);
            Some(resize_dims(cw, ch, st.resize_height)?)
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
        let yuv = !matches!(st.color, Color::None | Color::Rgb32);
        let matrix = if yuv { st.matrix } else { Matrix::Default };
        // Honour the checkbox even on Rec709/Rec601/Rec2020 (warned, not silently dropped).
        let range_convert = st.range_convert;
        let bt709_tags = yuv && matrix == Matrix::Rec709;
        let mut audio = st.audio.clone();
        audio.source_channels = crate::ui::codecs_ui::source_channels(
            self.tested.as_ref().and_then(|(_, r)| r.as_ref().ok()).or(self.probe.info()),
        );
        let pr = EncodeParams {
            script_path: script_path.clone(),
            script_text: text.clone(),
            output: output.clone(),
            video: st.video.clone(),
            audio,
            has_audio: st.audio_mode != AudioMode::None,
            pix_fmt: st.color.pix_fmt(conflicts::effective_bits(st.edit_script, &text, &st.video)),
            resize,
            bt709_tags,
            matrix,
            range_convert,
            chapters,
            faststart: st.faststart,
            picture,
        };
        let mut cmd = fs::build(&pr)?;
        cmd.warnings.extend(pic_warnings);
        for msg in fs::script_warnings(&st.options()) {
            if !cmd.warnings.iter().any(|w| w == &msg) {
                cmd.warnings.push(msg);
            }
        }
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
        self.st.migrate_bit_depth();
        self.st.migrate_forced_fps(v);
        self.probe = ProbeSlot::default();
        self.tested = None;
        self.pic = Default::default();
    }

    fn conflict_warnings(&self) -> Vec<ffenc::cmd::conflicts::Conflict> {
        conflicts::detect(&self.st.conflict_choices())
    }

    fn apply_conflict_fix(&mut self, kind: ConflictKind) {
        self.st.apply_conflict(kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presets saved with the removed script frame-rate option still load; the forced rate moves to
    /// the Output frame rate box when that is empty, otherwise it is ignored.
    #[test]
    fn old_forced_frame_rate_moves_to_output_frame_rate() {
        let s = Settings::default();
        let mut m = FrameserveMode::default();
        let base = m.save();
        let with = |extra: serde_json::Value| {
            let mut v = base.clone();
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };
        m.load(&with(serde_json::json!({"assume_fps": true, "fps_num": 24000, "fps_den": 1001})), &s);
        assert_eq!(m.st.video.output_fps, "24000/1001");
        m.load(&with(serde_json::json!({"assume_fps": true, "fps_num": 25, "fps_den": 1})), &s);
        assert_eq!(m.st.video.output_fps, "25");
        // the box already has a rate: keep it
        let mut v = with(serde_json::json!({"assume_fps": true, "fps_num": 60, "fps_den": 1}));
        v["video"]["output_fps"] = "30".into();
        m.load(&v, &s);
        assert_eq!(m.st.video.output_fps, "30");
        // option was off (or broken values): ignored
        m.load(&with(serde_json::json!({"assume_fps": false, "fps_num": 60, "fps_den": 1})), &s);
        assert_eq!(m.st.video.output_fps, "");
        m.load(&with(serde_json::json!({"assume_fps": true, "fps_num": 0, "fps_den": 1})), &s);
        assert_eq!(m.st.video.output_fps, "");
        // the old keys are not written again, and the script has no frame-rate line
        assert!(m.save().get("assume_fps").is_none());
        assert!(!m.st.script().unwrap_or_default().contains("FPS("));
    }

    #[test]
    fn defaults_yuv420_and_paths() {
        let m = FrameserveMode::default();
        assert_eq!(m.st.color, Color::Yuv420, "default colour conversion is ConvertToYUV420()");
        assert_eq!(m.st.matrix, Matrix::Pc709, "new jobs default to PC.709");
        assert_eq!(m.st.place, Place::Temp);
        let mut st = State { signpost: r"/v/My Clip.avi".into(), ..State::default() };
        st.regen_name();
        assert_eq!(st.out_name, "My Clip_frameserved.mp4");
        let t = st.script().unwrap();
        assert!(t.contains("ConvertToYUV420(matrix=\"PC.709\")"), "{t}");
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
        assert_eq!(m.st.video.codec, ffenc::cmd::codecs::VideoCodec::H264);
        assert_eq!(m.st.audio.codec, ffenc::cmd::codecs::AudioCodec::Aac);
        assert!(!m.st.range_convert);
        assert!(!m.st.faststart, "new jobs default faststart off");
    }

    #[test]
    fn omitted_matrix_stays_rec709_and_new_jobs_are_pc709() {
        assert_eq!(FrameserveMode::default().st.matrix, Matrix::Pc709);
        let v = serde_json::json!({
            "signpost": r"C:\clips\a.avi",
            "color": "Yuv420"
        });
        let mut m = FrameserveMode::default();
        m.load(&v, &Settings::default());
        assert_eq!(m.st.matrix, Matrix::Rec709, "old presets that omitted matrix keep Rec709");
        let mut m = FrameserveMode::default();
        m.load(&serde_json::json!({ "matrix": "Rec709" }), &Settings::default());
        assert_eq!(m.st.matrix, Matrix::Rec709, "an explicitly saved Rec709 matrix is unchanged");
        let mut m = FrameserveMode::default();
        m.load(&serde_json::json!({ "matrix": "Pc709" }), &Settings::default());
        assert_eq!(m.st.matrix, Matrix::Pc709);
    }

    #[test]
    fn old_json_loads_as_h264_aac_and_no_range_convert() {

        let v = serde_json::json!({
            "signpost": r"C:\clips\a.avi",
            "out_dir": r"D:\Videos\Out",
            "out_name": "a_frameserved.mp4",
            "last_auto": "a_frameserved.mp4",
            "suffix": "_frameserved",
            "container": "mp4",
            "color": "Yuv420",
            "matrix": "Rec709",
            "bits": "Eight",
            "video": { "crf": 22, "preset": "slow", "tune": null },
            "audio": { "bitrate_k": 192 }
        });
        let mut m = FrameserveMode::default();
        m.load(&v, &Settings::default());
        assert_eq!(m.st.video.codec, ffenc::cmd::codecs::VideoCodec::H264, "missing codec field defaults to H.264");
        assert_eq!(m.st.audio.codec, ffenc::cmd::codecs::AudioCodec::Aac, "missing codec field defaults to AAC");
        assert_eq!(m.st.video.crf, 22);
        assert_eq!(m.st.audio.bitrate_k, 192);
        assert!(!m.st.range_convert, "old presets do not enable a range squeeze");
        assert!(m.st.faststart, "old presets that omitted faststart keep it on");
        assert_eq!(m.st.matrix, Matrix::Rec709);
        let o = m.st.options();
        let pr = EncodeParams {
            script_path: PathBuf::from(r"C:\T\ffenc_frameserve_1.avs"),
            script_text: fs::script(&o).unwrap(),
            output: PathBuf::from(r"C:\clips\a_frameserved.mp4"),
            video: m.st.video.clone(),
            audio: m.st.audio.clone(),
            has_audio: true,
            pix_fmt: m.st.color.pix_fmt(m.st.bits),
            resize: None,
            bt709_tags: true,
            matrix: m.st.matrix,
            range_convert: m.st.range_convert,
            chapters: None,
            faststart: m.st.faststart,
            picture: Default::default(),
        };
        let b = fs::build(&pr).unwrap();
        assert_eq!(
            b.args,
            [
                "-y", "-i", r"C:\T\ffenc_frameserve_1.avs", "-map", "0:v:0", "-map", "0:a:0?",
                "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p",
                "-color_range", "tv", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
                "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", r"C:\clips\a_frameserved.mp4"
            ]
        );
        assert!(!b.args.iter().any(|a| a.contains("in_range=full")));
    }

    fn msgs(st: &State) -> Vec<String> {
        conflicts::messages(&conflicts::detect(&st.conflict_choices()))
    }

    #[test]
    fn unusual_choices_warn_and_do_not_change_the_form() {
        use ffenc::cmd::codecs::VideoCodec;
        let mut st = State::default();
        st.signpost = r"C:\clips\a.avi".into();
        assert_eq!(st.bits, BitDepth::Eight);
        assert!(!st.range_convert);
        assert!(msgs(&st).is_empty(), "safe defaults: {:?}", msgs(&st));

        // Picking HEVC: the codec 10-bit box (default on) also sets the script to 10-bit
        st.video.codec = VideoCodec::Hevc;
        st.bits = conflicts::script_bit_depth(&st.video);
        assert!(st.video.ten_bit);
        assert_eq!(st.options().bits, BitDepth::Ten);
        let text = fs::script(&st.options()).unwrap();
        assert!(text.contains("ConvertBits(10)"), "{text}");
        assert!(msgs(&st).iter().all(|m| !m.contains("encoder has 10-bit")), "{:?}", msgs(&st));

        // YV12 + 10-bit: not rewritten; warning; script still builds
        st.color = Color::Yv12;
        st.signpost = r"C:\clips\a.avi".into();
        assert_eq!(st.options().bits, BitDepth::Ten);
        let w = msgs(&st);
        assert!(w.iter().any(|m| m.contains("ConvertToYV12") && m.contains("still run")), "{w:?}");
        assert!(st.options().color == Color::Yv12);
        let text = fs::script(&st.options()).unwrap();
        assert!(text.contains("ConvertToYV12") && text.contains("ConvertBits(10)"));

        // Rec709 + range_convert: not cleared; warning
        st.color = Color::Yuv420;
        st.matrix = Matrix::Rec709;
        st.range_convert = true;
        let w = msgs(&st);
        assert!(st.range_convert);
        assert!(w.iter().any(|m| m.contains("washed out") && m.contains("still run")), "{w:?}");
    }

    #[test]
    fn fix_buttons_change_only_the_intended_field() {
        use ffenc::cmd::codecs::VideoCodec;
        let mut st = State::default();
        st.color = Color::Yv12;
        st.video.codec = VideoCodec::Hevc;
        st.video.ten_bit = true;
        st.signpost = r"C:\clips\a.avi".into();
        st.bits = conflicts::script_bit_depth(&st.video);
        assert!(msgs(&st).iter().any(|m| m.contains("ConvertToYV12")));
        let color_before = st.color;
        let ten_before = st.video.ten_bit;
        st.apply_conflict(ConflictKind::TenBitEightBitColor);
        assert_eq!(st.color, Color::Yuv420);
        assert_eq!(st.video.ten_bit, ten_before);
        assert_ne!(st.color, color_before);
        assert!(msgs(&st).iter().all(|m| !m.contains("ConvertToYV12")), "{:?}", msgs(&st));

        st.matrix = Matrix::Rec709;
        st.range_convert = true;
        assert!(msgs(&st).iter().any(|m| m.contains("washed out")));
        st.apply_conflict(ConflictKind::DoubleRangeSqueeze);
        assert!(!st.range_convert);
        assert!(msgs(&st).iter().all(|m| !m.contains("washed out")), "{:?}", msgs(&st));
    }

    #[test]
    fn old_script_bits_map_onto_codec_10_bit() {
        use ffenc::cmd::codecs::VideoCodec;
        // Old HEVC preset with script bits=Ten → 10-bit on
        let v = serde_json::json!({
            "signpost": r"C:\clips\a.avi",
            "out_name": "a_frameserved.mp4",
            "last_auto": "a_frameserved.mp4",
            "container": "mp4",
            "color": "Yuv420",
            "matrix": "Rec709",
            "bits": "Ten",
            "video": { "codec": "Hevc", "crf": 20, "preset": "slow", "ten_bit": false },
            "audio": { "bitrate_k": 192 }
        });
        let mut m = FrameserveMode::default();
        m.load(&v, &Settings::default());
        assert_eq!(m.st.video.codec, VideoCodec::Hevc);
        assert!(m.st.video.ten_bit, "script bits=Ten maps to the codec 10-bit box");
        assert_eq!(m.st.options().bits, BitDepth::Ten);
        assert!(fs::script(&m.st.options()).unwrap().contains("ConvertBits(10)"));

        // Old HEVC preset with script bits=Eight → 10-bit off
        let v = serde_json::json!({
            "signpost": r"C:\clips\a.avi",
            "out_name": "a_frameserved.mp4",
            "container": "mp4",
            "color": "Yuv420",
            "bits": "Eight",
            "video": { "codec": "Hevc", "crf": 20, "preset": "slow", "ten_bit": true },
            "audio": { "bitrate_k": 192 }
        });
        m.load(&v, &Settings::default());
        assert!(!m.st.video.ten_bit, "script bits=Eight maps to the codec 10-bit box off");
        assert_eq!(m.st.options().bits, BitDepth::Eight);
        assert!(!fs::script(&m.st.options()).unwrap().contains("ConvertBits(10)"));

        // Old H.264 + bits=Ten: H.264 stays 8-bit
        let v = serde_json::json!({
            "signpost": r"C:\clips\a.avi",
            "out_name": "a_frameserved.mp4",
            "container": "mp4",
            "color": "Yuv420",
            "bits": "Ten",
            "video": { "crf": 22, "preset": "slow" },
            "audio": { "bitrate_k": 192 }
        });
        m.load(&v, &Settings::default());
        assert_eq!(m.st.video.codec, VideoCodec::H264);
        assert_eq!(m.st.options().bits, BitDepth::Eight);
        assert!(!fs::script(&m.st.options()).unwrap().contains("ConvertBits(10)"));
    }

    #[test]
    fn hand_edited_script_bit_depth_mismatch_warns_and_fix_edits_script() {
        use ffenc::cmd::codecs::VideoCodec;
        let mut st = State::default();
        st.signpost = r"C:\clips\a.avi".into();
        st.video.codec = VideoCodec::Hevc;
        st.video.ten_bit = true;
        st.edit_script = true;
        st.script_text = "AviSource(\"C:\\\\clips\\\\a.avi\")\r\nConvertToYUV420(matrix=\"Rec709\")\r\n".into();
        let w = msgs(&st);
        assert!(w.iter().any(|m| m.contains("encoder has 10-bit") && m.contains("still run")), "{w:?}");
        st.apply_conflict(ConflictKind::EncoderTenBitScriptEight);
        assert!(conflicts::script_is_10bit(&st.script_text), "{}", st.script_text);
        assert!(st.video.ten_bit);
        assert!(msgs(&st).iter().all(|m| !m.contains("encoder has 10-bit")), "{:?}", msgs(&st));
    }
}
