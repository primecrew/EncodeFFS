//! "Add Watermark" (1.1): put an image on one or more videos, with the full Re-encode video and
//! audio choices (audio is copied as-is by default). Each file is built with `reencode::build`,
//! the crop / watermark resolved against that file's own size.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use crate::ui::option_tips;
use crate::ui::picture::{self, PicSource, PictureRt};
use ffenc::cmd::codecs::AudioCodec;
use ffenc::cmd::common::*;
use ffenc::cmd::picture::PictureSettings;
use ffenc::cmd::reencode::{self, ReencodeParams};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{has_ext, stem, ANY_VIDEO_EXTS};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    /// Videos of this run (not saved in the preset).
    files: Vec<String>,
    out_dir: String,
    suffix: String,
    /// "auto" = .mkv for .mkv input, else .mp4
    container: String,
    video: VideoEncode,
    audio: AudioEncode,
    resize: bool,
    resize_height: u32,
    range_convert: bool,
    deinterlace: bool,
    faststart: bool,
    picture: PictureSettings,
}

impl State {
    fn from_settings(s: &Settings) -> Self {
        let mut picture = PictureSettings::default();
        picture.watermark.enabled = true;
        State {
            files: vec![],
            out_dir: s.output_folder.clone(),
            suffix: "_watermarked".into(),
            container: "auto".into(),
            video: default_video(s),
            audio: AudioEncode { codec: AudioCodec::Copy, ..default_audio(s) },
            resize: false,
            resize_height: 720,
            range_convert: false,
            deinterlace: true,
            faststart: false,
            picture,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

enum Probe {
    Busy(Bg<Result<MediaInfo, String>>),
    Done(Result<MediaInfo, String>),
}

#[derive(Default)]
pub struct WatermarkMode {
    st: State,
    probes: HashMap<PathBuf, Probe>,
    pic: PictureRt,
}

impl WatermarkMode {
    fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            let sp = s(&p);
            if p.is_file() && has_ext(&p, ANY_VIDEO_EXTS) && !self.st.files.contains(&sp) {
                self.st.files.push(sp);
            }
        }
    }
    fn poll(&mut self, cx: &Ctx) {
        for f in &self.st.files {
            let p = PathBuf::from(f);
            match self.probes.get(&p) {
                None => {
                    let pr = match cx.ffprobe {
                        Some(fp) => {
                            let (fp, q) = (fp.to_path_buf(), p.clone());
                            Probe::Busy(Bg::spawn(cx.egui, move || probe::probe(&fp, &q)))
                        }
                        None => Probe::Done(Err("FFprobe not found – set the FFmpeg folder in Settings.".into())),
                    };
                    self.probes.insert(p, pr);
                }
                Some(Probe::Busy(b)) => {
                    if let Some(r) = b.poll() {
                        self.probes.insert(p, Probe::Done(r));
                    }
                }
                Some(Probe::Done(_)) => {}
            }
        }
    }
    fn info(&self, f: &str) -> Option<&Result<MediaInfo, String>> {
        match self.probes.get(Path::new(f)) {
            Some(Probe::Done(r)) => Some(r),
            _ => None,
        }
    }
    fn sources(&self) -> Vec<PicSource> {
        self.st.files.iter().filter_map(|f| self.info(f).and_then(|r| r.as_ref().ok()).and_then(|i| PicSource::from_info(Path::new(f), i))).collect()
    }
    fn output_for(&self, input: &Path) -> Result<PathBuf, String> {
        let ext = if self.st.container == "auto" { reencode::default_output_ext(input).to_string() } else { self.st.container.clone() };
        let dir = if self.st.out_dir.trim().is_empty() { input.parent().map(s).unwrap_or_default() } else { self.st.out_dir.clone() };
        check_output(&dir, &format!("{}{}.{}", stem(input), self.st.suffix, ext))
    }
}

impl Mode for WatermarkMode {
    fn key(&self) -> &'static str {
        "watermark"
    }
    fn title(&self) -> &'static str {
        "Add Watermark"
    }
    fn description(&self) -> &'static str {
        "Put an image (logo, name) on one or more videos: drag it into place, resize it, set the opacity. Audio is copied as-is by default."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["files"]
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        self.poll(cx);
        self.st.picture.watermark.enabled = true;
        let hovering = files_hovering(ui.ctx());

        // ---- 1. videos
        section(ui, "1. Videos");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add files…").clicked() {
                let mut d = rfd::FileDialog::new().add_filter("Video", ANY_VIDEO_EXTS).add_filter("All files", &["*"]);
                if !cx.settings.input_folder.is_empty() {
                    d = d.set_directory(&cx.settings.input_folder);
                }
                if let Some(mut v) = d.pick_files() {
                    ffenc::util::natural_sort_paths(&mut v);
                    self.add(v);
                }
            }
            if ui.button("Add folder…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.add(ffenc::util::expand_dropped(&[d]));
                }
            }
            if ui.add_enabled(!self.st.files.is_empty(), egui::Button::new("Clear")).clicked() {
                self.st.files.clear();
            }
        });
        let frame = egui::Frame::group(ui.style()).inner_margin(6.0);
        let out = frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if self.st.files.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Drop videos or a folder here, or use Add files…").size(15.0));
                    hint(ui, "Drop an image here to use it as the watermark.");
                });
                ui.add_space(12.0);
                return;
            }
            let mut remove = None;
            egui::ScrollArea::vertical().max_height(180.0).id_salt("wm_files").show(ui, |ui| {
                for (i, f) in self.st.files.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if icon_button(ui, true, "✖", &format!("Remove file {} from the list", i + 1)).clicked() {
                            remove = Some(i);
                        }
                        ui.label(Path::new(f).file_name().unwrap_or_default().to_string_lossy()).on_hover_text(f);
                        match self.info(f) {
                            Some(Ok(info)) => hint(ui, describe_video(info)),
                            Some(Err(e)) => {
                                ui.colored_label(crate::ui::theme::c().err, e);
                            }
                            None => {
                                ui.spinner();
                            }
                        }
                    });
                }
            });
            if let Some(i) = remove {
                self.st.files.remove(i);
            }
        });
        if hovering {
            paint_drop_target(ui, out.response.rect);
        }

        let srcs = self.sources();
        // ---- 2. watermark (the editor is this preset's own screen)
        section(ui, "2. Watermark");
        picture::watermark_editor(ui, cx, "wmk", &mut self.st.picture, &srcs, &mut self.pic);

        // ---- 3. crop (optional)
        ui.add_space(6.0);
        picture::crop_section(ui, cx, "wmk", &mut self.st.picture.crop, &srcs, &mut self.pic);

        // ---- 4. encode
        let st = &mut self.st;
        let first = st.files.first().and_then(|f| self.probes.get(Path::new(f))).and_then(|p| match p {
            Probe::Done(Ok(i)) => Some(i),
            _ => None,
        });
        section(ui, &format!("3. Video ({})", st.video.codec.encoder()));
        let pix = crate::ui::codecs_ui::source_pix_fmt(first);
        egui::Grid::new("wm_v").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::codecs_ui::video_codec_ui(ui, "wmv", &mut st.video, pix.as_deref(), &cx.ff_status().with_height(crate::ui::codecs_ui::source_height(first)));
            crate::ui::common::row_label_tip(ui, "Resize", option_tips::RESIZE);
            ui.horizontal(|ui| {
                ui.checkbox(&mut st.resize, "New vertical resolution");
                if st.resize {
                    num_u32(ui, &mut st.resize_height, 1..=4320, " px");
                    for h in [480, 720, 1080, 1440, 2160] {
                        if ui.small_button(h.to_string()).clicked() {
                            st.resize_height = h;
                        }
                    }
                }
            });
            ui.end_row();
            range_ui(ui, &mut st.range_convert, first.map(|i| i.video_color_range()));
            crate::ui::common::row_label_tip(ui, "Deinterlace", option_tips::DEINTERLACE);
            ui.checkbox(&mut st.deinterlace, "bwdif double-rate deinterlace for files detected as interlaced");
            ui.end_row();
        });
        section(ui, "4. Audio");
        egui::Grid::new("wm_a").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::codecs_ui::audio_codec_ui(ui, "wma", &mut st.audio, true, &cx.ff_status().with_height(crate::ui::codecs_ui::source_height(first)));
        });
        section(ui, "5. Output");
        egui::Grid::new("wm_o").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            folder_row_nodrop(ui, "wm_out", "Output folder", &mut st.out_dir, &cx.settings.output_folder, &cx.settings.recent_folders, "(same as source folder)");
            crate::ui::common::row_label(ui, "Name suffix");
            ui.text_edit_singleline(&mut st.suffix);
            ui.end_row();
            crate::ui::common::row_label_tip(ui, "Container", option_tips::CONTAINER);
            egui::ComboBox::from_id_salt("wm_cont").selected_text(st.container.clone()).show_ui(ui, |ui| {
                for c in ["auto", "mp4", "mkv", "mov"] {
                    ui.selectable_value(&mut st.container, c.to_string(), if c == "auto" { "auto (MKV in -> MKV, else MP4)" } else { c });
                }
            });
            ui.end_row();
            faststart_ui(ui, &mut st.faststart, st.container != "mkv");
        });
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        let st = &self.st;
        if st.files.is_empty() {
            return Err("Add at least one video.".into());
        }
        let img = st.picture.watermark.image.trim();
        if img.is_empty() {
            return Err("Choose the watermark image.".into());
        }
        if !Path::new(img).is_file() {
            return Err(format!("Watermark image not found: {img}"));
        }
        parse_output_fps(&st.video.output_fps)?;
        crate::ui::codecs_ui::check_available(cx, Some(&st.video), Some(&st.audio))?;
        let img_size = self.pic.image_size(&st.picture.watermark);
        let mut jobs = vec![];
        for f in &st.files {
            let input = PathBuf::from(f);
            let name = input.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let info = match self.info(f) {
                Some(Ok(i)) => i,
                Some(Err(e)) => return Err(format!("{name}: {e}")),
                None => return Err(format!("{name}: still probing, try again in a moment")),
            };
            let v = info.video_summary().map_err(|e| format!("{name}: {e}"))?;
            let output = self.output_for(&input)?;
            if same_file(&input, &output) {
                return Err(format!("{name}: the output would overwrite the input. Use a suffix or another folder."));
            }
            let (pic, warns) = st.picture.resolve(v.width, v.height, img_size).map_err(|e| format!("{name}: {e}"))?;
            let (cw, ch) = pic.cropped_size(v.width, v.height);
            let resize = if st.resize { Some(resize_dims(cw, ch, st.resize_height)?) } else { None };
            let mut audio = st.audio.clone();
            audio.source_channels = crate::ui::codecs_ui::source_channels(Some(info));
            let pr = ReencodeParams {
                input: input.clone(),
                output: output.clone(),
                duration_s: v.duration_s,
                trim_start: 0.0,
                trim_end: 0.0,
                passthrough: Passthrough::None,
                video: st.video.clone(),
                audio,
                range_convert: st.range_convert,
                resize,
                deinterlace: st.deinterlace && v.interlaced,
                chapters: None,
                faststart: st.faststart,
                subtitle_streams: info.subtitle_refs(),
                has_attachments: info.has_attachments(),
                picture: pic,
            };
            let mut cmd = reencode::build(&pr).map_err(|e| format!("{name}: {e}"))?;
            cmd.warnings.extend(warns.into_iter().map(|w| format!("{name}: {w}")));
            jobs.push(JobSpec::new(format!("Watermark {name}"), cmd, output.clone(), log_path_for(&output), Some(pr.output_duration())));
        }
        let n = jobs.len();
        Ok(Plan { jobs, group: (n > 1).then(|| ("Add Watermark".into(), false, 0)), notes: if n > 1 { vec![format!("Prepared {n} watermark job(s).")] } else { vec![] } })
    }

    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        let mut v: Vec<PathBuf> = paths.iter().filter(|p| has_ext(p, ANY_VIDEO_EXTS)).cloned().collect();
        ffenc::util::natural_sort_paths(&mut v);
        self.add(v);
        if let Some(img) = paths.iter().find(|p| has_ext(p, picture::WM_IMAGE_EXTS)) {
            self.st.picture.watermark.image = s(img);
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }

    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        let base = State::from_settings(s);
        let v = if v.is_object() { ffenc::presets::overlay(serde_json::to_value(&base).unwrap_or_default(), v) } else { v.clone() };
        self.st = restore(&v, base);
        self.st.picture.watermark.enabled = true;
        self.probes.clear();
        self.pic = Default::default();
    }

    fn conflict_warnings(&self) -> Vec<ffenc::cmd::conflicts::Conflict> {
        let name = self.st.files.first().map(|f| format!("{}{}.{}", stem(Path::new(f)), self.st.suffix, if self.st.container == "auto" { reencode::default_output_ext(Path::new(f)).to_string() } else { self.st.container.clone() })).unwrap_or_else(|| format!("out.{}", if self.st.container == "auto" { "mp4" } else { &self.st.container }));
        let out = PathBuf::from(self.st.out_dir.trim()).join(&name);
        ffenc::cmd::conflicts::detect(&super::media_choices(&self.st.video, &self.st.audio, &self.st.container, &name, &name, out, self.st.range_convert, None))
    }

    fn apply_conflict_fix(&mut self, kind: ffenc::cmd::conflicts::ConflictKind) {
        let mut name = format!("out.{}", if self.st.container == "auto" { "mp4" } else { &self.st.container });
        let mut last = name.clone();
        let out = PathBuf::from(&name);
        super::apply_media_fix(kind, &mut self.st.video, &mut self.st.audio, &mut self.st.container, &mut name, &mut last, out, Some(&mut self.st.range_convert));
    }
}
