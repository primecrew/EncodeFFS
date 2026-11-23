//! "Audio & subtitles" – one easy tool for a video's audio and subtitle tracks:
//! keep / remove / rename / reorder, add commentary or another language, replace audio,
//! add or remove subtitles, and extract audio or subtitles to separate files.
//! Builders: `ffenc::cmd::tracks`.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::common::{CompressorParams, CompressorPreset};
use ffenc::cmd::tracks::{self as tr, AudioFormat, AudioTrack, Container, Fit, Length, Source, SubFormat, SubTrack, TracksParams};
use ffenc::ffmpeg::probe::MediaInfo;
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{stem, ANY_AUDIO_EXTS, ANY_VIDEO_EXTS, SUB_EXTS};

/// Subtitle files we accept (plus subtitle tracks of other videos).
const SUB_FILE_EXTS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "mks"];
/// Audio files (plus the audio of other videos).
const AUDIO_FILE_EXTS: &[&str] = &["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus", "mka", "ac3", "eac3", "wma"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
enum AddKind {
    #[default]
    Commentary,
    Language,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    video: String,
    /// The video the track list below belongs to.
    tracks_of: String,
    audio: Vec<AudioTrack>,
    subs: Vec<SubTrack>,
    make_video: bool,
    container: Container,
    out_dir: String,
    out_name: String,
    last_auto: String,
    suffix: String,
    #[serde(default = "ffenc::cmd::common::missing_faststart")]
    faststart: bool,
    /// Shared values used by any added file track set to Advanced.
    #[serde(default)]
    advanced: CompressorParams,
    add_kind: AddKind,
    add_title: String,
    add_lang: String,
    /// -1 = all original tracks, otherwise index into `audio`.
    replace_target: i64,
    sub_lang: String,
    extract_audio: bool,
    extract_audio_all: bool,
    extract_audio_track: usize,
    audio_format: AudioFormat,
    extract_subs: bool,
    extract_subs_all: bool,
    extract_sub_track: usize,
    sub_format: SubFormat,
    /// Empty = the output folder.
    extract_dir: String,
}

impl State {
    fn from_settings(s: &Settings) -> Self {
        State {
            video: String::new(),
            tracks_of: String::new(),
            audio: vec![],
            subs: vec![],
            make_video: true,
            container: Container::Mkv,
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: "_tracks".into(),
            faststart: false,
            advanced: s.compressor,
            add_kind: AddKind::Commentary,
            add_title: s.commentary_name.clone(),
            add_lang: "eng".into(),
            replace_target: -1,
            sub_lang: "eng".into(),
            extract_audio: false,
            extract_audio_all: false,
            extract_audio_track: 0,
            audio_format: AudioFormat::Wav,
            extract_subs: false,
            extract_subs_all: true,
            extract_sub_track: 0,
            sub_format: SubFormat::Srt,
            extract_dir: String::new(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

/// What a file being probed is for.
#[derive(Clone)]
enum Purpose {
    AddAudio { title: String, lang: String },
    /// None = replace every original track.
    Replace(Option<usize>),
    AddSubs { lang: String },
}

struct Pending {
    path: PathBuf,
    purpose: Purpose,
    task: Bg<Result<MediaInfo, String>>,
}

#[derive(Default)]
pub struct TracksMode {
    st: State,
    probe: ProbeSlot,
    pending: Vec<Pending>,
    /// Last message about added files (shown under the sections).
    note: Option<(String, bool)>,
}

enum RowAct {
    None,
    Up,
    Down,
    Remove,
    Default,
}

fn video_count(i: &MediaInfo) -> usize {
    i.of_type("video").filter(|v| !v.attached_pic).count()
}

fn set_video(st: &mut State, p: &Path) {
    st.video = s(p);
    if let Some(c) = Container::of(p) {
        st.container = c;
    }
    let name = format!("{}{}.{}", stem(p), st.suffix, st.container.ext());
    auto_name(&mut st.out_name, &mut st.last_auto, name);
}

/// Output name with the container's extension.
fn with_ext(name: &str, c: Container) -> String {
    let p = PathBuf::from(name.trim());
    if Container::of(&p) == Some(c) {
        return name.trim().to_string();
    }
    let st = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // ".mkv" alone has no real name: leave it empty so the automatic name is used.
    if st.trim().is_empty() || st.trim_start_matches('.').is_empty() || name.trim().starts_with('.') && Container::of(&PathBuf::from(format!("x{}", name.trim()))).is_some() {
        return String::new();
    }
    format!("{st}.{}", c.ext())
}

/// "AAC · 2 ch" / "SRT text"
fn audio_desc(t: &AudioTrack) -> String {
    let mut s = tr::audio_codec_name(&t.codec);
    if let Some(c) = t.channels {
        s.push_str(&format!(" · {}", match c {
            1 => "mono".to_string(),
            2 => "stereo".to_string(),
            6 => "5.1".to_string(),
            8 => "7.1".to_string(),
            n => format!("{n} channels"),
        }));
    }
    s
}

fn source_desc(src: &Source) -> String {
    match src {
        Source::Main => "from the video".into(),
        Source::File(f) => format!("added from {}", f.file_name().unwrap_or_default().to_string_lossy()),
    }
}

/// Label + single-line field, linked for screen readers.
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32, hint_text: &str) -> egui::Response {
    let l = ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(width).hint_text(hint_text)).labelled_by(l.id)
}

fn ms_field(ui: &mut egui::Ui, label: &str, v: &mut i64, name: &str) {
    let l = ui.label(label);
    let r = ui.add(egui::DragValue::new(v).range(-600_000..=600_000).speed(10.0).suffix(" ms")).labelled_by(l.id);
    named(r, egui::WidgetType::DragValue, name);
}

fn move_buttons(ui: &mut egui::Ui, i: usize, n: usize, what: &str) -> RowAct {
    let mut act = RowAct::None;
    if icon_button(ui, i > 0, "⬆", &format!("Move {what} up")).clicked() {
        act = RowAct::Up;
    }
    if icon_button(ui, i + 1 < n, "⬇", &format!("Move {what} down")).clicked() {
        act = RowAct::Down;
    }
    act
}

impl TracksMode {
    fn info(&self) -> Option<&MediaInfo> {
        self.probe.info()
    }

    /// Replace the video's own tracks with what ffprobe found (added tracks stay).
    fn load_main_tracks(&mut self) {
        let Some(info) = self.probe.info().cloned() else { return };
        let st = &mut self.st;
        st.audio.retain(|t| t.source != Source::Main);
        st.subs.retain(|t| t.source != Source::Main);
        let main_a: Vec<AudioTrack> = info
            .of_type("audio")
            .enumerate()
            .map(|(k, a)| AudioTrack {
                stream: k,
                codec: a.codec_name.clone(),
                channels: a.channels,
                duration: a.duration,
                title: a.title.clone().unwrap_or_default(),
                language: a.language.clone().unwrap_or_default(),
                default: a.default,
                ..Default::default()
            })
            .collect();
        let main_s: Vec<SubTrack> = info
            .of_type("subtitle")
            .enumerate()
            .map(|(k, s)| SubTrack {
                stream: k,
                codec: s.codec_name.clone(),
                title: s.title.clone().unwrap_or_default(),
                language: s.language.clone().unwrap_or_default(),
                default: s.default,
                forced: s.forced,
                ..Default::default()
            })
            .collect();
        st.audio.splice(0..0, main_a);
        st.subs.splice(0..0, main_s);
        st.tracks_of = st.video.trim().to_string();
        if st.out_name.trim().is_empty() {
            let v = PathBuf::from(st.video.trim());
            set_video(st, &v);
        }
        st.extract_audio_track = 0;
        st.extract_sub_track = 0;
        st.replace_target = -1;
    }

    fn add_files(&mut self, paths: Vec<PathBuf>, purpose: Purpose, cx: &Ctx) {
        let Some(fp) = cx.ffprobe.map(|p| p.to_path_buf()) else {
            self.note = Some(("FFprobe was not found – set the FFmpeg folder in File › Settings.".into(), false));
            return;
        };
        for p in paths {
            let f = fp.clone();
            let q = p.clone();
            let task = Bg::spawn(cx.egui, move || ffenc::ffmpeg::probe::probe(&f, &q));
            self.pending.push(Pending { path: p, purpose: purpose.clone(), task });
        }
    }

    /// Collect finished probes of added files.
    fn poll_pending(&mut self) {
        let mut i = 0;
        while i < self.pending.len() {
            let Some(r) = self.pending[i].task.poll() else {
                i += 1;
                continue;
            };
            let pd = self.pending.remove(i);
            let name = pd.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let info = match r {
                Ok(info) => info,
                Err(e) => {
                    self.note = Some((format!("Could not read {name}: {e}"), false));
                    continue;
                }
            };
            let dur = info.any_duration();
            match pd.purpose {
                Purpose::AddAudio { title, lang } => {
                    let streams: Vec<_> = info.of_type("audio").cloned().collect();
                    if streams.is_empty() {
                        self.note = Some((format!("{name} has no audio – nothing was added."), false));
                        continue;
                    }
                    let many = streams.len() > 1;
                    for (k, a) in streams.iter().enumerate() {
                        let t = if many { a.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| format!("{title} {}", k + 1)) } else { title.clone() };
                        let l = if many { a.language.clone().filter(|l| l != "und").unwrap_or(lang.clone()) } else { lang.clone() };
                        self.st.audio.push(AudioTrack {
                            source: Source::File(pd.path.clone()),
                            stream: k,
                            codec: a.codec_name.clone(),
                            channels: a.channels,
                            duration: a.duration.or(dur),
                            title: t,
                            language: l,
                            ..Default::default()
                        });
                    }
                    self.note = Some((format!("Added {} audio track{} from {name}.", streams.len(), if many { "s" } else { "" }), true));
                }
                Purpose::Replace(target) => {
                    let Some(a) = info.of_type("audio").next().cloned() else {
                        self.note = Some((format!("{name} has no audio – nothing was replaced."), false));
                        continue;
                    };
                    let mut new = AudioTrack {
                        source: Source::File(pd.path.clone()),
                        codec: a.codec_name.clone(),
                        channels: a.channels,
                        duration: a.duration.or(dur),
                        ..Default::default()
                    };
                    let st = &mut self.st;
                    let pos = match target.filter(|&t| t < st.audio.len()) {
                        Some(t) => {
                            let old = &mut st.audio[t];
                            new.title = old.title.clone();
                            new.language = old.language.clone();
                            new.default = old.default;
                            old.keep = false;
                            old.default = false;
                            t
                        }
                        None => {
                            let first = st.audio.iter().find(|t| t.source == Source::Main).cloned();
                            for t in st.audio.iter_mut().filter(|t| t.source == Source::Main) {
                                t.keep = false;
                                t.default = false;
                            }
                            if let Some(f) = first {
                                new.title = f.title;
                                new.language = f.language;
                            }
                            new.default = true;
                            0
                        }
                    };
                    st.audio.insert(pos, new);
                    self.note = Some((format!("{name} replaces {}. The original stays in the list, unticked – tick it again to keep it as well.", if target.is_some() { "that track" } else { "all original audio" }), true));
                }
                Purpose::AddSubs { lang } => {
                    let streams: Vec<_> = info.of_type("subtitle").cloned().collect();
                    if streams.is_empty() {
                        self.note = Some((format!("{name} has no subtitles ffmpeg can read – nothing was added."), false));
                        continue;
                    }
                    let many = streams.len() > 1;
                    for (k, sbt) in streams.iter().enumerate() {
                        self.st.subs.push(SubTrack {
                            source: Source::File(pd.path.clone()),
                            stream: k,
                            codec: sbt.codec_name.clone(),
                            duration: dur,
                            title: sbt.title.clone().unwrap_or_default(),
                            language: if many { sbt.language.clone().unwrap_or(lang.clone()) } else { lang.clone() },
                            forced: sbt.forced,
                            ..Default::default()
                        });
                    }
                    self.note = Some((format!("Added {} subtitle track{} from {name}.", streams.len(), if many { "s" } else { "" }), true));
                }
            }
        }
    }

    fn add_audio_purpose(&self) -> Purpose {
        Purpose::AddAudio { title: self.st.add_title.clone(), lang: self.st.add_lang.clone() }
    }

    fn tracks_ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let vdur = self.info().and_then(|i| i.duration());
        let tol = cx.settings.duration_tolerance_s;
        let mp4 = self.st.container == Container::Mp4 && self.st.make_video;
        let st = &mut self.st;

        // ---- audio
        ui.label(RichText::new(format!("Audio tracks ({})", st.audio.len())).strong());
        if st.audio.is_empty() {
            hint(ui, "No audio tracks. Add one below, or pick a video first.");
        }
        let n = st.audio.len();
        let mut act: Option<(usize, RowAct)> = None;
        let kept_before: Vec<usize> = st.audio.iter().scan(0, |c, t| {
            if t.keep {
                *c += 1;
            }
            Some(*c)
        }).collect();
        for i in 0..n {
            let t = &mut st.audio[i];
            let name = if t.title.trim().is_empty() { format!("audio track {}", i + 1) } else { format!("audio track {} ({})", i + 1, t.title.trim()) };
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    let r = ui.checkbox(&mut t.keep, RichText::new(format!("Audio {}", i + 1)).strong());
                    named(r, egui::WidgetType::Checkbox, &format!("Keep {name}"));
                    ui.label(format!("{} · {}", audio_desc(t), source_desc(&t.source)));
                    if t.keep {
                        hint(ui, format!("(becomes output audio {})", kept_before[i]));
                    } else {
                        ui.colored_label(crate::ui::theme::c().warn, "removed");
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    field(ui, "Title", &mut t.title, 180.0, "e.g. Commentary Track");
                    field(ui, "Language", &mut t.language, 60.0, "eng");
                    let r = ui.radio(t.default, "Default");
                    if named(r, egui::WidgetType::RadioButton, &format!("Make {name} the default (plays first)")).clicked() {
                        act = Some((i, RowAct::Default));
                    }
                    let m = move_buttons(ui, i, n, &name);
                    if !matches!(m, RowAct::None) {
                        act = Some((i, m));
                    }
                    if t.source != Source::Main && icon_button(ui, true, "✖", &format!("Take {name} off the list")).clicked() {
                        act = Some((i, RowAct::Remove));
                    }
                });
                if mp4 && t.keep && !tr::audio_fits(Container::Mp4, &t.codec) {
                    hint(ui, format!("MP4 can't hold {} audio as it is – it will be converted to AAC.", tr::audio_codec_name(&t.codec)));
                }
                if let Source::File(_) = &t.source {
                    ui.horizontal_wrapped(|ui| {
                        ms_field(ui, "Delay", &mut t.delay_ms, &format!("Delay for {name} in milliseconds"));
                        hint(ui, "positive = starts later (use when it plays too early); negative = skips the start of the file");
                    });
                    ui.horizontal_wrapped(|ui| {
                        let l = ui.label("Volume");
                        let r = ui.add(egui::DragValue::new(&mut t.gain_db).range(-30.0..=30.0).speed(0.1).suffix(" dB")).labelled_by(l.id);
                        named(r, egui::WidgetType::DragValue, &format!("Volume change for {name} in decibels"));
                        let l = ui.label("Compressor");
                        egui::ComboBox::from_id_salt(("comp", i)).selected_text(t.compressor.label()).show_ui(ui, |ui| {
                            for c in CompressorPreset::ALL {
                                ui.selectable_value(&mut t.compressor, c, format!("{} – {}", c.label(), c.help()));
                            }
                        }).response.labelled_by(l.id);
                        if t.compressor == CompressorPreset::Advanced {
                            hint(ui, "Advanced controls are below this list (shared by every track set to Advanced).");
                        }
                    });
                    if let (Some(v), Some(d)) = (vdur, t.duration) {
                        let (diff, verdict) = tr::length_check(v, d, t.delay_ms, tol);
                        let note = tr::length_note(v, d, t.delay_ms, tol);
                        if verdict == Length::Matches {
                            hint(ui, note);
                        } else {
                            warn(ui, note);
                        }
                        if diff > 0.05 {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("It is longer than the video:");
                                let trim = matches!(t.fit, Fit::Auto | Fit::Trim | Fit::Leave | Fit::Pad);
                                if ui.radio(trim, "Trim audio to the video length").clicked() {
                                    t.fit = Fit::Trim;
                                }
                                if ui.radio(t.fit == Fit::KeepFull, "Keep the full audio (the file may run longer than the picture)").clicked() {
                                    t.fit = Fit::KeepFull;
                                }
                            });
                        } else if diff < -0.05 {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("It is shorter than the video:");
                                if ui.radio(t.fit != Fit::Pad, "Leave as is (the audio just ends early; the video is not cut)").clicked() {
                                    t.fit = Fit::Leave;
                                }
                                if ui.radio(t.fit == Fit::Pad, "Pad with silence to the video length").clicked() {
                                    t.fit = Fit::Pad;
                                }
                            });
                        }
                    }
                    let r = ui.checkbox(&mut t.mix_with_main, "Also add a track that mixes this into the main audio");
                    named(r, egui::WidgetType::Checkbox, &format!("Also add a track that mixes {name} into the main audio (for players that play only one track)"));
                }
            });
        }
        if let Some((i, a)) = act {
            match a {
                RowAct::Up if i > 0 => st.audio.swap(i, i - 1),
                RowAct::Down if i + 1 < st.audio.len() => st.audio.swap(i, i + 1),
                RowAct::Remove => {
                    st.audio.remove(i);
                }
                RowAct::Default => {
                    let was = st.audio[i].default;
                    for t in st.audio.iter_mut() {
                        t.default = false;
                    }
                    st.audio[i].default = !was;
                }
                _ => {}
            }
        }

        if st.audio.iter().any(|t| t.compressor == CompressorPreset::Advanced) {
            ui.add_space(4.0);
            ui.label(RichText::new("Advanced compressor").strong());
            hint(ui, "These values apply to every added file track set to Advanced. Volume on the track row is pre-gain, applied before the compressor.");
            compressor_params_ui(ui, "tracks_adv", &mut st.advanced);
        }

        // ---- subtitles
        ui.add_space(6.0);
        ui.label(RichText::new(format!("Subtitle tracks ({})", st.subs.len())).strong());
        if st.subs.is_empty() {
            hint(ui, "No subtitle tracks. Add subtitle files below.");
        }
        let n = st.subs.len();
        let mut act: Option<(usize, RowAct)> = None;
        for i in 0..n {
            let t = &mut st.subs[i];
            let name = if t.title.trim().is_empty() { format!("subtitle track {}", i + 1) } else { format!("subtitle track {} ({})", i + 1, t.title.trim()) };
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    let r = ui.checkbox(&mut t.keep, RichText::new(format!("Subtitles {}", i + 1)).strong());
                    named(r, egui::WidgetType::Checkbox, &format!("Keep {name}"));
                    ui.label(format!("{} · {}", tr::sub_codec_name(&t.codec), source_desc(&t.source)));
                    if !t.keep {
                        ui.colored_label(crate::ui::theme::c().warn, "removed");
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    field(ui, "Title", &mut t.title, 180.0, "e.g. English (SDH)");
                    field(ui, "Language", &mut t.language, 60.0, "eng");
                    let r = ui.radio(t.default, "Default");
                    if named(r, egui::WidgetType::RadioButton, &format!("Make {name} the default (shown automatically)")).clicked() {
                        act = Some((i, RowAct::Default));
                    }
                    let r = ui.checkbox(&mut t.forced, "Forced");
                    named(r, egui::WidgetType::Checkbox, &format!("Forced: {name} is shown even when subtitles are off (foreign-language parts)"));
                    let m = move_buttons(ui, i, n, &name);
                    if !matches!(m, RowAct::None) {
                        act = Some((i, m));
                    }
                    if t.source != Source::Main && icon_button(ui, true, "✖", &format!("Take {name} off the list")).clicked() {
                        act = Some((i, RowAct::Remove));
                    }
                });
                if mp4 && t.keep {
                    if !tr::is_text_sub(&t.codec) {
                        warn(ui, "This is a picture subtitle – MP4 can't hold it, so it will be left out. Choose MKV below to keep it.");
                    } else if matches!(t.codec.as_str(), "ass" | "ssa") {
                        hint(ui, "In MP4 this becomes plain text (styling is lost).");
                    }
                }
                if let Source::File(f) = &t.source {
                    ui.horizontal_wrapped(|ui| {
                        ms_field(ui, "Delay", &mut t.delay_ms, &format!("Delay for {name} in milliseconds"));
                        hint(ui, "positive = later (use when lines appear too early)");
                    });
                    if tr::is_text_sub(&t.codec) {
                        ui.horizontal_wrapped(|ui| {
                            let l = ui.label("Text encoding");
                            let cur = tr::CHARSETS.iter().find(|c| c.0.eq_ignore_ascii_case(&t.charset)).map(|c| c.1).unwrap_or(t.charset.as_str()).to_string();
                            egui::ComboBox::from_id_salt(("cs", i)).selected_text(cur).show_ui(ui, |ui| {
                                for (code, label) in tr::CHARSETS {
                                    ui.selectable_value(&mut t.charset, code.to_string(), *label);
                                }
                            }).response.labelled_by(l.id);
                            if t.charset.is_empty() {
                                let detected = std::fs::read(f).ok().map(|b| tr::detect_charset(&b));
                                match detected {
                                    Some(None) => hint(ui, "detected: UTF-8"),
                                    Some(Some(_)) => hint(ui, "not UTF-8 – read as Western (Windows-1252); pick another if letters look wrong"),
                                    None => {}
                                }
                            }
                        });
                    }
                }
            });
        }
        if let Some((i, a)) = act {
            match a {
                RowAct::Up if i > 0 => st.subs.swap(i, i - 1),
                RowAct::Down if i + 1 < st.subs.len() => st.subs.swap(i, i + 1),
                RowAct::Remove => {
                    st.subs.remove(i);
                }
                RowAct::Default => {
                    let was = st.subs[i].default;
                    for t in st.subs.iter_mut() {
                        t.default = false;
                    }
                    st.subs[i].default = !was;
                }
                _ => {}
            }
        }
    }

    fn params(&self, info: &MediaInfo, out: PathBuf, cx: &Ctx) -> TracksParams {
        TracksParams {
            video: PathBuf::from(self.st.video.trim()),
            video_duration: info.duration(),
            audio: self.st.audio.clone(),
            subs: self.st.subs.clone(),
            output: out,
            aac_bitrate_k: cx.settings.audio_bitrate_k,
            advanced: self.st.advanced,
            tolerance_s: cx.settings.duration_tolerance_s,
            faststart: self.st.faststart,
        }
    }

    fn pick_files(title: &str, exts: &[&str], dir: &str) -> Vec<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title(title);
        if Path::new(dir).is_dir() {
            d = d.set_directory(dir);
        }
        d.add_filter(title, exts).add_filter("All files", &["*"]).pick_files().unwrap_or_default()
    }
}

impl Mode for TracksMode {
    fn key(&self) -> &'static str {
        "tracks"
    }
    fn title(&self) -> &'static str {
        "Audio & subtitles"
    }
    fn description(&self) -> &'static str {
        "See a video's audio and subtitle tracks; keep, remove, rename, reorder; add commentary, another language or subtitle files; replace or extract audio. The picture is copied, never re-encoded."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["video", "tracks_of", "audio", "subs", "out_name", "last_auto"]
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let arrived = self.probe.update(cx.egui, cx.ffprobe, &self.st.video);
        if arrived && self.probe.info().is_some() && self.st.tracks_of != self.st.video.trim() {
            self.load_main_tracks();
        }
        self.poll_pending();
        let in_dir = cx.settings.input_folder.clone();

        egui::Grid::new("tracks_video").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Video", &mut self.st.video, Pick::File("Video", ANY_VIDEO_EXTS), &in_dir) {
                let p = PathBuf::from(self.st.video.trim());
                set_video(&mut self.st, &p);
            }
            ui.label("");
            self.probe.status_ui(ui, describe_video);
            ui.end_row();
        });
        if self.st.video.trim().is_empty() {
            hint(ui, "Pick a video (or drop one on the window) to see its audio and subtitle tracks. You can drop audio and subtitle files too.");
        }
        if !self.pending.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Reading {} file(s)…", self.pending.len()));
            });
        }
        if let Some((t, ok)) = &self.note {
            let c = if *ok { crate::ui::theme::c().ok } else { crate::ui::theme::c().err };
            ui.colored_label(c, t);
        }

        section(ui, "Tracks in the new file");
        hint(ui, "Untick a track to remove it. Title and language are what players show. Use the arrows to change the order.");
        self.tracks_ui(ui, cx);
        if let Some(info) = self.info() {
            if self.st.make_video {
                let out = PathBuf::from(with_ext("x", self.st.container));
                let pr = TracksParams { output: out, ..self.params(info, PathBuf::new(), cx) };
                ui.label(RichText::new(tr::summary(&pr, video_count(info))).strong());
            }
        }

        // ---- actions
        let st = &mut self.st;
        let mut add_audio = false;
        let mut replace: Option<Option<usize>> = None;
        let mut add_subs = false;
        crate::ui::layout::section(ui, "tracks.add_audio", egui::CollapsingHeader::new("Add an audio track (commentary or another language)"), false, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.radio(st.add_kind == AddKind::Commentary, "Commentary").clicked() {
                    st.add_kind = AddKind::Commentary;
                    st.add_title = cx.settings.commentary_name.clone();
                }
                if ui.radio(st.add_kind == AddKind::Language, "Another language / second audio").clicked() {
                    st.add_kind = AddKind::Language;
                    st.add_title = "Second audio".into();
                }
            });
            ui.horizontal_wrapped(|ui| {
                field(ui, "Title for the new track", &mut st.add_title, 200.0, "Commentary Track");
                field(ui, "Language", &mut st.add_lang, 60.0, "eng");
            });
            if ui.button("Choose audio file(s)…").clicked() {
                add_audio = true;
            }
            hint(ui, "WAV, MP3, FLAC, M4A, AAC, OGG or OPUS – or another video, to take its audio. You can also drop audio files on the window. Delay, volume and a mixed version can be set on the new track above.");
        });
        crate::ui::layout::section(ui, "tracks.replace_audio", egui::CollapsingHeader::new("Replace audio"), false, |ui| {
            let mains: Vec<(usize, String)> = st.audio.iter().enumerate().filter(|(_, t)| t.source == Source::Main).map(|(i, t)| (i, format!("Audio {}{}", i + 1, if t.title.is_empty() { String::new() } else { format!(" ({})", t.title) }))).collect();
            if mains.is_empty() {
                hint(ui, "The video has no audio of its own – use “Add an audio track”.");
            } else {
                ui.horizontal_wrapped(|ui| {
                    let l = ui.label("Replace");
                    let cur = if st.replace_target < 0 { "All original audio tracks".to_string() } else { mains.iter().find(|m| m.0 as i64 == st.replace_target).map(|m| m.1.clone()).unwrap_or_else(|| "All original audio tracks".into()) };
                    egui::ComboBox::from_id_salt("replace_target").selected_text(cur).show_ui(ui, |ui| {
                        ui.selectable_value(&mut st.replace_target, -1, "All original audio tracks");
                        for (i, m) in &mains {
                            ui.selectable_value(&mut st.replace_target, *i as i64, m);
                        }
                    }).response.labelled_by(l.id);
                    if ui.button("Choose the new audio file…").clicked() {
                        replace = Some((st.replace_target >= 0).then_some(st.replace_target as usize));
                    }
                });
                hint(ui, "The new track takes the old one's place, title and language. The old one stays in the list, unticked.");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Remove all original audio").clicked() {
                        for t in st.audio.iter_mut().filter(|t| t.source == Source::Main) {
                            t.keep = false;
                        }
                    }
                    if ui.button("Keep all original audio").clicked() {
                        for t in st.audio.iter_mut().filter(|t| t.source == Source::Main) {
                            t.keep = true;
                        }
                    }
                });
            }
        });
        crate::ui::layout::section(ui, "tracks.add_subtitles", egui::CollapsingHeader::new("Add subtitles"), false, |ui| {
            ui.horizontal_wrapped(|ui| {
                field(ui, "Language of the new subtitles", &mut st.sub_lang, 60.0, "eng");
                if ui.button("Choose subtitle file(s)…").clicked() {
                    add_subs = true;
                }
            });
            hint(ui, "SRT, ASS/SSA, VTT, SUB/IDX or SUP files – or another video, to take its subtitles. Title, default, forced, delay and text encoding can be set on each new track above.");
        });
        crate::ui::layout::section(ui, "tracks.remove_subtitles", egui::CollapsingHeader::new("Remove subtitles"), false, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Remove all subtitles").clicked() {
                    for t in st.subs.iter_mut() {
                        t.keep = false;
                    }
                }
                if ui.button("Keep all subtitles").clicked() {
                    for t in st.subs.iter_mut() {
                        t.keep = true;
                    }
                }
            });
            hint(ui, "To remove only some, untick them in the list above. To save a subtitle track as a file, use “Extract subtitles” below.");
        });
        let main_audio: Vec<(usize, String)> = st.audio.iter().filter(|t| t.source == Source::Main).map(|t| (t.stream, format!("Audio {}{} – {}", t.stream + 1, if t.title.is_empty() { String::new() } else { format!(" ({})", t.title) }, audio_desc(t)))).collect();
        crate::ui::layout::section(ui, "tracks.extract_audio", egui::CollapsingHeader::new("Extract audio to separate files"), false, |ui| {
            ui.checkbox(&mut st.extract_audio, "Save audio from the video as separate files");
            ui.add_enabled_ui(st.extract_audio, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.radio(!st.extract_audio_all, "One track:").clicked() {
                        st.extract_audio_all = false;
                    }
                    let cur = main_audio.iter().find(|m| m.0 == st.extract_audio_track).map(|m| m.1.clone()).unwrap_or_else(|| "(pick a video first)".into());
                    egui::ComboBox::from_id_salt("xa_track").selected_text(cur).show_ui(ui, |ui| {
                        for (k, m) in &main_audio {
                            ui.selectable_value(&mut st.extract_audio_track, *k, m);
                        }
                    });
                    if ui.radio(st.extract_audio_all, "All audio tracks").clicked() {
                        st.extract_audio_all = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let l = ui.label("Format");
                    egui::ComboBox::from_id_salt("xa_fmt").selected_text(st.audio_format.label()).show_ui(ui, |ui| {
                        for f in AudioFormat::ALL {
                            ui.selectable_value(&mut st.audio_format, f, f.label());
                        }
                    }).response.labelled_by(l.id);
                });
                if let Some(t) = st.audio.iter().find(|t| t.source == Source::Main && t.stream == st.extract_audio_track) {
                    hint(ui, format!("File names look like: {}", tr::extract_name(Path::new(st.video.trim()), "audio", t.stream + 1, &t.language, &t.title, tr::audio_ext(st.audio_format, &t.codec))));
                }
            });
        });
        let main_subs: Vec<(usize, String)> = st.subs.iter().filter(|t| t.source == Source::Main).map(|t| (t.stream, format!("Subtitles {}{} – {}", t.stream + 1, if t.title.is_empty() { format!(" ({})", t.language) } else { format!(" ({})", t.title) }, tr::sub_codec_name(&t.codec)))).collect();
        crate::ui::layout::section(ui, "tracks.extract_subtitles", egui::CollapsingHeader::new("Extract subtitles to separate files"), false, |ui| {
            ui.checkbox(&mut st.extract_subs, "Save subtitles from the video as separate files");
            ui.add_enabled_ui(st.extract_subs, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.radio(!st.extract_subs_all, "One track:").clicked() {
                        st.extract_subs_all = false;
                    }
                    let cur = main_subs.iter().find(|m| m.0 == st.extract_sub_track).map(|m| m.1.clone()).unwrap_or_else(|| "(no subtitles)".into());
                    egui::ComboBox::from_id_salt("xs_track").selected_text(cur).show_ui(ui, |ui| {
                        for (k, m) in &main_subs {
                            ui.selectable_value(&mut st.extract_sub_track, *k, m);
                        }
                    });
                    if ui.radio(st.extract_subs_all, "All subtitle tracks").clicked() {
                        st.extract_subs_all = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let l = ui.label("Format");
                    egui::ComboBox::from_id_salt("xs_fmt").selected_text(st.sub_format.label()).show_ui(ui, |ui| {
                        for f in SubFormat::ALL {
                            ui.selectable_value(&mut st.sub_format, f, f.label());
                        }
                    }).response.labelled_by(l.id);
                });
            });
        });
        if st.extract_audio || st.extract_subs {
            egui::Grid::new("tracks_xdir").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                path_row_nodrop(ui, "Folder for extracted files", &mut st.extract_dir, Pick::Folder, &cx.settings.output_folder);
            });
            hint(ui, "Empty = the output folder below.");
        }

        section(ui, "New video file");
        ui.checkbox(&mut st.make_video, "Make a new video with the tracks above");
        ui.add_enabled_ui(st.make_video, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Container");
                for c in [Container::Mkv, Container::Mp4] {
                    if ui.radio(st.container == c, c.label()).clicked() && st.container != c {
                        st.container = c;
                        let n = with_ext(&st.out_name, c);
                        if st.out_name == st.last_auto {
                            st.last_auto = n.clone();
                        }
                        st.out_name = n;
                    }
                }
            });
            hint(ui, st.container.explain());
            egui::Grid::new("tracks_out").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                if output_rows_sfx(ui, &mut st.out_dir, &mut st.out_name, &mut st.suffix, &cx.settings.output_folder) && !st.video.trim().is_empty() {
                    let p = PathBuf::from(st.video.trim());
                    set_video(st, &p);
                }
                faststart_ui(ui, &mut st.faststart, st.container == Container::Mp4);
            });
        });

        let dir = if self.st.video.trim().is_empty() { in_dir.clone() } else { PathBuf::from(self.st.video.trim()).parent().map(s).unwrap_or(in_dir.clone()) };
        if add_audio {
            let mut exts: Vec<&str> = AUDIO_FILE_EXTS.to_vec();
            exts.extend(ANY_VIDEO_EXTS);
            let files = Self::pick_files("Audio or video files", &exts, &dir);
            let p = self.add_audio_purpose();
            self.add_files(files, p, cx);
        }
        if let Some(target) = replace {
            let mut exts: Vec<&str> = AUDIO_FILE_EXTS.to_vec();
            exts.extend(ANY_VIDEO_EXTS);
            let files = Self::pick_files("New audio file", &exts, &dir);
            self.add_files(files.into_iter().take(1).collect(), Purpose::Replace(target), cx);
        }
        if add_subs {
            let mut exts: Vec<&str> = SUB_FILE_EXTS.to_vec();
            exts.extend(ANY_VIDEO_EXTS);
            let files = Self::pick_files("Subtitle files", &exts, &dir);
            let lang = self.st.sub_lang.clone();
            self.add_files(files, Purpose::AddSubs { lang }, cx);
        }
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        if !self.pending.is_empty() {
            return Err("Still reading the added files – try again in a moment (still probing).".into());
        }
        let st = &self.st;
        let video = check_input(&st.video, ANY_VIDEO_EXTS, "video")?;
        let info = self.probe.require("Video")?.clone();
        if st.tracks_of != st.video.trim() {
            return Err("Video: still probing, try again in a moment".into());
        }
        if !st.make_video && !st.extract_audio && !st.extract_subs {
            return Err("Nothing to do: tick “Make a new video”, “Extract audio” or “Extract subtitles”.".into());
        }
        let mut jobs = vec![];
        let mut notes = vec![];
        if st.make_video {
            let mut name = with_ext(&st.out_name, st.container);
            if name.is_empty() {
                name = format!("{}{}.{}", stem(&video), st.suffix, st.container.ext());
            }
            let output = check_output(&st.out_dir, &name)?;
            let pr = self.params(&info, output.clone(), cx);
            let cmd = tr::build(&pr)?;
            notes.push(tr::summary(&pr, video_count(&info)));
            if let Some(v) = pr.video_duration {
                for (i, t) in pr.audio.iter().enumerate().filter(|(_, t)| t.keep) {
                    if let (Source::File(_), Some(d)) = (&t.source, t.duration) {
                        let diff = d + t.delay_ms as f64 / 1000.0 - v;
                        let what = match tr::effective_fit(t.fit, diff) {
                            Fit::Trim => " It will be trimmed to the video length.",
                            Fit::Pad => " It will be padded with silence to the video length.",
                            Fit::KeepFull if diff > 0.05 => " It is kept in full, so the file runs a bit longer than the picture.",
                            _ => "",
                        };
                        notes.push(format!("Audio {} ({}): {}{what}", i + 1, t.title, tr::length_note(v, d, t.delay_ms, pr.tolerance_s)));
                    }
                }
            }
            jobs.push(JobSpec::new(format!("Audio & subtitles -> {name}"), cmd, output.clone(), log_path_for(&output), info.duration()));
        }
        let xdir = if st.extract_dir.trim().is_empty() { st.out_dir.clone() } else { st.extract_dir.clone() };
        let mains_a: Vec<&AudioTrack> = st.audio.iter().filter(|t| t.source == Source::Main).collect();
        if st.extract_audio {
            let pick: Vec<&AudioTrack> = mains_a.iter().copied().filter(|t| st.extract_audio_all || t.stream == st.extract_audio_track).collect();
            if pick.is_empty() {
                return Err("Extract audio: the video has no audio track.".into());
            }
            for t in pick {
                let name = tr::extract_name(&video, "audio", t.stream + 1, &t.language, &t.title, tr::audio_ext(st.audio_format, &t.codec));
                let out = check_output(&xdir, &name)?;
                let cmd = tr::extract_audio(&video, t.stream, &t.codec, st.audio_format, cx.settings.audio_bitrate_k, &out);
                jobs.push(JobSpec::new(format!("Extract audio {} -> {name}", t.stream + 1), cmd, out.clone(), log_path_for(&out), info.duration()));
            }
        }
        if st.extract_subs {
            let pick: Vec<&SubTrack> = st.subs.iter().filter(|t| t.source == Source::Main && (st.extract_subs_all || t.stream == st.extract_sub_track)).collect();
            if pick.is_empty() {
                return Err("Extract subtitles: the video has no subtitle track.".into());
            }
            for t in pick {
                let (ext, _) = match tr::sub_target(st.sub_format, &t.codec) {
                    Ok(x) => x,
                    Err(e) => {
                        notes.push(format!("Subtitles {} skipped: {e}", t.stream + 1));
                        continue;
                    }
                };
                let name = tr::extract_name(&video, "subtitles", t.stream + 1, &t.language, &t.title, ext);
                let out = check_output(&xdir, &name)?;
                let cmd = tr::extract_sub(&video, t.stream, &t.codec, st.sub_format, &out)?;
                jobs.push(JobSpec::new(format!("Extract subtitles {} -> {name}", t.stream + 1), cmd, out.clone(), log_path_for(&out), None));
            }
        }
        if jobs.is_empty() {
            return Err(format!("Nothing to do. {}", notes.join(" ")));
        }
        Ok(Plan { jobs, group: None, notes, cpu_only: false })
    }

    fn on_drop(&mut self, paths: &[PathBuf], cx: &Ctx) {
        let videos: Vec<&PathBuf> = paths.iter().filter(|p| ffenc::util::has_ext(p, ANY_VIDEO_EXTS)).collect();
        let audio: Vec<PathBuf> = paths.iter().filter(|p| ffenc::util::has_ext(p, ANY_AUDIO_EXTS) || ffenc::util::has_ext(p, AUDIO_FILE_EXTS)).cloned().collect();
        let subs: Vec<PathBuf> = paths.iter().filter(|p| ffenc::util::has_ext(p, SUB_EXTS) || ffenc::util::has_ext(p, SUB_FILE_EXTS)).cloned().collect();
        if let Some(v) = videos.first() {
            set_video(&mut self.st, v);
        }
        if !audio.is_empty() {
            let p = self.add_audio_purpose();
            self.add_files(audio, p, cx);
        }
        if !subs.is_empty() {
            let lang = self.st.sub_lang.clone();
            self.add_files(subs, Purpose::AddSubs { lang }, cx);
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }

    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        self.probe = ProbeSlot::default();
        self.pending.clear();
        self.note = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_extension_follows_container() {
        assert_eq!(with_ext("clip_tracks.mkv", Container::Mp4), "clip_tracks.mp4");
        assert_eq!(with_ext("clip.v2_tracks.mp4", Container::Mkv), "clip.v2_tracks.mkv");
        assert_eq!(with_ext("clip.mp4", Container::Mp4), "clip.mp4");
        assert_eq!(with_ext("", Container::Mkv), "");
        assert_eq!(with_ext(".mkv", Container::Mp4), "");
        let mut st = State::default();
        set_video(&mut st, Path::new("/v/My clip.mp4"));
        assert_eq!((st.container, st.out_name.as_str()), (Container::Mp4, "My clip_tracks.mp4"));
        // run fields stay out of presets; options survive a save/load round trip
        let m = TracksMode { st, ..Default::default() };
        let v = m.save();
        for k in m.run_fields() {
            assert!(v.get(*k).is_some(), "{k}");
        }
        let mut m2 = TracksMode::default();
        m2.load(&v, &Settings::default());
        assert_eq!(m2.st.out_name, "My clip_tracks.mp4");
    }
}
