//! Create Slideshow – still images → H.264 MP4 with optional audio and crossfades.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui::{self, ColorImage, RichText, TextureHandle, TextureOptions};
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::common::{VideoEncode, PRESETS};
use ffenc::cmd::slideshow::{
    self, DurationMode, ImageInput, OutputSize, SmallImage, SlideshowParams, Transition, IMAGE_EXTS,
};
use ffenc::ffmpeg::probe;
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, natural_cmp, natural_sort_paths, stem, AUDIO_EXTS};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    images: Vec<String>,
    audio: String,
    out_dir: String,
    out_name: String,
    last_auto: String,
    #[serde(default = "sfx")]
    suffix: String,
    duration_mode: DurationMode,
    per_image_s: f64,
    fixed_total_s: f64,
    transition: Transition,
    fade_s: f64,
    size: OutputSize,
    small: SmallImage,
    fps: u32,
    video: VideoEncode,
    audio_bitrate_k: u32,
    faststart: bool,
}

impl State {
    fn from_settings(s: &Settings) -> Self {
        let mut video = default_video(s);
        video.crf = 18;
        video.preset = "medium".into();
        video.output_fps.clear();
        State {
            images: vec![],
            audio: String::new(),
            out_dir: s.output_folder.clone(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: sfx(),
            duration_mode: DurationMode::PerImage,
            per_image_s: 5.0,
            fixed_total_s: 60.0,
            transition: Transition::Fade,
            fade_s: 1.0,
            size: OutputSize::S1920x1080,
            small: SmallImage::BlackBorder,
            fps: 30,
            video,
            audio_bitrate_k: default_audio(s).bitrate_k,
            faststart: true,
        }
    }
}
impl Default for State {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

fn sfx() -> String {
    "_slideshow".into()
}

enum DimProbe {
    Busy(Bg<Result<(u32, u32), String>>),
    Done(Result<(u32, u32), String>),
}

struct Thumb {
    tex: TextureHandle,
    /// Source mtime when loaded (reload if the file changes).
    mtime: SystemTime,
}

#[derive(Default)]
pub struct SlideshowMode {
    st: State,
    dims: HashMap<PathBuf, DimProbe>,
    thumbs: HashMap<PathBuf, Thumb>,
    audio_probe: ProbeSlot,
    /// Drag-reorder source index while dragging a reel cell.
    drag_from: Option<usize>,
}

fn modified(p: &Path) -> SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

fn load_thumb(ctx: &egui::Context, path: &Path) -> Option<Thumb> {
    let mtime = modified(path);
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = img.thumbnail(160, 120);
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    let tex = ctx.load_texture(format!("ss_{}", path.display()), color, TextureOptions::LINEAR);
    Some(Thumb { tex, mtime })
}

impl SlideshowMode {
    fn add_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut added = false;
        for p in paths {
            if p.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|f| f.is_file() && slideshow::is_image(f)).collect();
                    natural_sort_paths(&mut files);
                    for f in files {
                        let sp = s(&f);
                        if !self.st.images.contains(&sp) {
                            self.st.images.push(sp);
                            added = true;
                        }
                    }
                }
            } else if p.is_file() && slideshow::is_image(&p) {
                let sp = s(&p);
                if !self.st.images.contains(&sp) {
                    self.st.images.push(sp);
                    added = true;
                }
            }
        }
        if added {
            self.update_name();
        }
    }

    fn update_name(&mut self) {
        if let Some(first) = self.st.images.first() {
            let base = Path::new(first).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| stem(Path::new(first)));
            auto_name(&mut self.st.out_name, &mut self.st.last_auto, format!("{}{}.mp4", base, self.st.suffix));
        }
    }

    fn ensure_dims(&mut self, cx: &Ctx) {
        let ffprobe = cx.ffprobe.map(|p| p.to_path_buf());
        for path_s in &self.st.images {
            let p = PathBuf::from(path_s);
            match self.dims.get(&p) {
                Some(DimProbe::Busy(bg)) => {
                    if let Some(r) = bg.poll() {
                        self.dims.insert(p, DimProbe::Done(r));
                    }
                }
                Some(DimProbe::Done(_)) => {}
                None => {
                    if let Some(fp) = ffprobe.clone() {
                        let path = p.clone();
                        let bg = Bg::spawn(cx.egui, move || {
                            let info = probe::probe(&fp, &path)?;
                            let v = info.streams.iter().find(|s| s.codec_type == "video").ok_or_else(|| "not an image".to_string())?;
                            let (w, h) = (v.width.unwrap_or(0), v.height.unwrap_or(0));
                            if w == 0 || h == 0 {
                                return Err(format!("no dimensions ({w}×{h})"));
                            }
                            Ok((w, h))
                        });
                        self.dims.insert(p, DimProbe::Busy(bg));
                    }
                }
            }
        }
        // drop probes for removed images
        self.dims.retain(|p, _| self.st.images.iter().any(|s| Path::new(s) == p));
        self.thumbs.retain(|p, _| self.st.images.iter().any(|s| Path::new(s) == p));
    }

    fn image_inputs(&self) -> Result<Vec<ImageInput>, String> {
        let mut out = Vec::with_capacity(self.st.images.len());
        for path_s in &self.st.images {
            let p = PathBuf::from(path_s);
            match self.dims.get(&p) {
                Some(DimProbe::Done(Ok((w, h)))) => out.push(ImageInput { path: p, width: *w, height: *h }),
                Some(DimProbe::Done(Err(e))) => return Err(format!("{}: {e}", Path::new(path_s).file_name().unwrap_or_default().to_string_lossy())),
                Some(DimProbe::Busy(_)) | None => {
                    return Err("Still reading image sizes – try again in a moment.".into());
                }
            }
        }
        Ok(out)
    }

    fn reel_ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        let n = self.st.images.len();
        ui.horizontal(|ui| {
            if ui.button("Add images…").on_hover_text("Pick one or more image files.").clicked() {
                if let Some(files) = rfd::FileDialog::new().add_filter("Images", IMAGE_EXTS).set_directory(&cx.settings.input_folder).pick_files() {
                    self.add_paths(files);
                }
            }
            if ui.button("Add folder…").on_hover_text("Import every supported image in a folder (natural name order).").clicked() {
                if let Some(dir) = rfd::FileDialog::new().set_directory(&cx.settings.input_folder).pick_folder() {
                    self.add_paths([dir]);
                }
            }
            if ui.add_enabled(n > 0, egui::Button::new("Sort by name")).on_hover_text("Natural order: img2 before img10").clicked() {
                self.st.images.sort_by(|a, b| natural_cmp(&stem(Path::new(a)), &stem(Path::new(b))).then_with(|| natural_cmp(a, b)));
            }
            if ui.add_enabled(n > 0, egui::Button::new("Sort by modified date")).on_hover_text("Oldest first").clicked() {
                self.st.images.sort_by_key(|a| modified(Path::new(a)));
            }
            if ui.add_enabled(n > 1, egui::Button::new("Reverse")).clicked() {
                self.st.images.reverse();
            }
            if ui.add_enabled(n > 0, egui::Button::new("Clear")).clicked() {
                self.st.images.clear();
            }
            ui.label(RichText::new(format!("{n} image{}", if n == 1 { "" } else { "s" })).weak());
        });
        hint(ui, "Camera reel: drag a frame to reorder, or use ⬆ ⬇. Drop a folder or images onto the window.");

        let frame = egui::Frame::group(ui.style()).inner_margin(6.0);
        frame.show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("ss_reel").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut remove = None;
                    let mut move_pair = None; // (from, before)
                    for i in 0..self.st.images.len() {
                        let path = PathBuf::from(&self.st.images[i]);
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let dim_txt = match self.dims.get(&path) {
                            Some(DimProbe::Done(Ok((w, h)))) => format!("{w}×{h}"),
                            Some(DimProbe::Busy(_)) | None => "…".into(),
                            Some(DimProbe::Done(Err(_))) => "!".into(),
                        };
                        // ensure thumb
                        let need = match self.thumbs.get(&path) {
                            Some(t) if t.mtime == modified(&path) => false,
                            _ => true,
                        };
                        if need {
                            if let Some(t) = load_thumb(cx.egui, &path) {
                                self.thumbs.insert(path.clone(), t);
                            }
                        }
                        let cell = egui::Frame::NONE.inner_margin(4.0).corner_radius(4.0).fill(ui.visuals().faint_bg_color);
                        let resp = cell.show(ui, |ui| {
                            ui.set_min_size(egui::vec2(120.0, 0.0));
                            // film sprocket decoration
                            let sprocket = ui.available_width();
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(sprocket, 8.0), egui::Sense::hover());
                            let c = ui.visuals().weak_text_color();
                            for k in 0..6 {
                                ui.painter().rect_filled(egui::Rect::from_center_size(egui::pos2(rect.left() + 10.0 + k as f32 * 18.0, rect.center().y), egui::vec2(8.0, 5.0)), 1.0, c);
                            }
                            if let Some(t) = self.thumbs.get(&path) {
                                ui.add(egui::Image::new((t.tex.id(), egui::vec2(112.0, 84.0))));
                            } else {
                                ui.allocate_ui(egui::vec2(112.0, 84.0), |ui| {
                                    ui.centered_and_justified(|ui| ui.label("?"));
                                });
                            }
                            let (rect2, _) = ui.allocate_exact_size(egui::vec2(sprocket, 8.0), egui::Sense::hover());
                            for k in 0..6 {
                                ui.painter().rect_filled(egui::Rect::from_center_size(egui::pos2(rect2.left() + 10.0 + k as f32 * 18.0, rect2.center().y), egui::vec2(8.0, 5.0)), 1.0, c);
                            }
                            ui.label(RichText::new(format!("{}. {name}", i + 1)).small()).on_hover_text(path.display().to_string());
                            ui.label(RichText::new(dim_txt).small().weak());
                            ui.horizontal(|ui| {
                                if icon_button(ui, i > 0, "⬆", &format!("Move image {} up", i + 1)).clicked() {
                                    move_pair = Some((i, i));
                                }
                                if icon_button(ui, i + 1 < n, "⬇", &format!("Move image {} down", i + 1)).clicked() {
                                    move_pair = Some((i, i + 2));
                                }
                                if icon_button(ui, true, "✖", &format!("Remove image {}", i + 1)).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }).response;
                        let drag = ui.interact(resp.rect, ui.id().with(("ss_drag", i)), egui::Sense::drag());
                        named(drag.clone(), egui::WidgetType::DragValue, &format!("Image {}: drag to reorder", i + 1));
                        if drag.drag_started() {
                            self.drag_from = Some(i);
                        }
                        if drag.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        }
                        if drag.drag_stopped() {
                            if let Some(from) = self.drag_from.take() {
                                // drop onto the cell under the pointer
                                if let Some(pos) = ui.ctx().pointer_interact_pos() {
                                    // approximate: find which cell contains pos — use our own rects via hover
                                    let _ = pos;
                                }
                                let _ = from;
                            }
                        }
                        // drop target: if something is being dragged and this cell is hovered
                        if let Some(from) = self.drag_from {
                            if drag.hovered() && from != i {
                                ui.painter().rect_stroke(resp.rect, 4.0, egui::Stroke::new(2.0_f32, ui.visuals().selection.bg_fill), egui::StrokeKind::Outside);
                                if ui.input(|inp| inp.pointer.any_released()) {
                                    move_pair = Some((from, if from < i { i + 1 } else { i }));
                                    self.drag_from = None;
                                }
                            }
                        }
                    }
                    if let Some(i) = remove {
                        self.st.images.remove(i);
                        self.update_name();
                    }
                    if let Some((from, before)) = move_pair {
                        move_item(&mut self.st.images, from, before);
                        self.drag_from = None;
                    }
                });
            });
            if self.st.images.is_empty() {
                ui.label(RichText::new("No images yet — Add images…, Add folder…, or drop files here.").weak());
            }
        });
    }
}

// re-export move_item from join
use super::join::move_item;

impl Mode for SlideshowMode {
    fn key(&self) -> &'static str {
        "slideshow"
    }
    fn title(&self) -> &'static str {
        "Create Slideshow"
    }
    fn description(&self) -> &'static str {
        "Images → H.264/AAC MP4 slideshow. Per-image length (default 5 s), fixed total, or match audio. Crossfades when FFmpeg has xfade; otherwise hard cuts."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["images", "audio", "out_name", "last_auto"]
    }
    fn wants_folders(&self) -> bool {
        true
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        self.ensure_dims(cx);
        self.audio_probe.update(cx.egui, cx.ffprobe, &self.st.audio);

        section(ui, "Images (camera reel)");
        self.reel_ui(ui, cx);

        egui::Grid::new("ss_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            path_row(ui, "Audio (optional)", &mut self.st.audio, Pick::File("Audio", AUDIO_EXTS), &cx.settings.input_folder);
            ui.label("");
            self.audio_probe.status_ui(ui, describe_audio);
            ui.end_row();

            if output_rows_sfx(ui, &mut self.st.out_dir, &mut self.st.out_name, &mut self.st.suffix, &cx.settings.output_folder) {
                self.update_name();
            }

            section(ui, "Duration");
            ui.end_row();
            row_label_tip(ui, "Mode", "How long the slideshow lasts. Per image (default): each image is shown for a set time. Fixed total: you set the whole length. Match audio: the length follows the audio file.");
            ui.vertical(|ui| {
                for m in DurationMode::ALL {
                    ui.radio_value(&mut self.st.duration_mode, m, m.label());
                }
            });
            ui.end_row();
            match self.st.duration_mode {
                DurationMode::PerImage => {
                    row_label(ui, "Seconds per image");
                    num_f64(ui, &mut self.st.per_image_s, 0.1..=3600.0, 0.1, " s");
                    ui.end_row();
                }
                DurationMode::FixedTotal => {
                    row_label(ui, "Total length");
                    num_f64(ui, &mut self.st.fixed_total_s, 0.1..=86400.0, 0.5, " s");
                    ui.end_row();
                }
                DurationMode::MatchAudio => {
                    row_label(ui, "Audio length");
                    ui.vertical(|ui| {
                        if let Some(d) = self.audio_probe.info().and_then(|i| i.any_duration()) {
                            ui.label(format!("{} (from the audio file)", fmt_hms(d)));
                        } else if self.st.audio.trim().is_empty() {
                            warn(ui, "Choose an audio file for Match audio length.");
                        } else {
                            hint(ui, "Reading audio length…");
                        }
                    });
                    ui.end_row();
                }
            }

            // live timing preview
            if let Ok(images) = self.image_inputs() {
                let pr = self.params_preview(images, cx);
                if let Ok(t) = slideshow::timing(&pr) {
                    row_label(ui, "Result");
                    let fade_note = if t.fade_s > 0.0 {
                        format!(" · total = N×{:.2}s − (N−1)×{:.2}s fade", t.per_image_s, t.fade_s)
                    } else {
                        String::new()
                    };
                    hint(ui, format!("Each image {:.2} s · total {}{fade_note}", t.per_image_s, fmt_hms(t.total_s)));
                    ui.end_row();
                }
            }

            section(ui, "Transition");
            ui.end_row();
            let xfade_ok = cx.has_xfade.unwrap_or(false);
            if cx.has_xfade == Some(false) {
                warn(ui, "This FFmpeg build has no xfade filter, so only hard cuts are available. Help › Check for FFmpeg updates… can download a full GPL build that includes crossfades.");
                ui.end_row();
                ui.label("");
            }
            row_label_tip(ui, "Between images", "Crossfades need the FFmpeg xfade filter. If it is missing, transitions fall back to hard cuts (you will see a warning).");
            ui.vertical(|ui| {
                for tr in Transition::ALL {
                    let enabled = !tr.needs_xfade() || xfade_ok || cx.has_xfade.is_none();
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.radio_value(&mut self.st.transition, tr, tr.label());
                    });
                }
                if self.st.transition.needs_xfade() && !xfade_ok && cx.has_xfade.is_some() {
                    // force to None visually for the run, keep user's choice stored for when they upgrade FFmpeg
                    hint(ui, "Selected transition needs xfade — this run will use hard cuts.");
                }
            });
            ui.end_row();
            row_label_tip(ui, "Crossfade length", "Overlap between two images. Must be shorter than each image's display time. With N images: total ≈ N×per-image − (N−1)×fade.");
            ui.add_enabled_ui(self.st.transition != Transition::None, |ui| {
                num_f64(ui, &mut self.st.fade_s, 0.0..=30.0, 0.05, " s");
            });
            ui.end_row();

            section(ui, "Size & encode");
            ui.end_row();
            row_label_tip(ui, "Output size", "Canvas size. Images keep their aspect ratio (letterbox/pillarbox with black bars). Auto uses the largest image, rounded to even dimensions.");
            egui::ComboBox::from_id_salt("ss_size").width(260.0).selected_text(self.st.size.label()).show_ui(ui, |ui| {
                for sz in OutputSize::ALL {
                    ui.selectable_value(&mut self.st.size, sz, sz.label());
                }
            });
            ui.end_row();
            row_label_tip(ui, "Smaller images", "When an image is smaller than the canvas: black border keeps it sharp; bicubic upscale fills more of the frame but softens it.");
            ui.vertical(|ui| {
                for sm in SmallImage::ALL {
                    ui.radio_value(&mut self.st.small, sm, sm.label());
                }
            });
            ui.end_row();
            row_label(ui, "Frame rate");
            ui.horizontal(|ui| {
                num_u32(ui, &mut self.st.fps, 1..=120, " fps");
                hint(ui, "Slideshow default is 30 (stills).");
            });
            ui.end_row();
            row_label(ui, "CRF");
            ui.horizontal(|ui| {
                num_u32(ui, &mut self.st.video.crf, 0..=51, "");
                hint(ui, "18 is the slideshow default (visually lossless for stills).");
            });
            ui.end_row();
            row_label(ui, "Preset");
            egui::ComboBox::from_id_salt("ss_preset").selected_text(self.st.video.preset.clone()).show_ui(ui, |ui| {
                for p in PRESETS {
                    ui.selectable_value(&mut self.st.video.preset, (*p).to_string(), (*p).to_string());
                }
            });
            ui.end_row();
            bitrate_ui(ui, "AAC bitrate", &mut self.st.audio_bitrate_k);
            faststart_ui(ui, &mut self.st.faststart, true);
        });
    }

    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        if self.st.images.is_empty() {
            return Err("Add at least one image.".into());
        }
        let images = self.image_inputs()?;
        let output = check_output(&self.st.out_dir, &self.st.out_name)?;
        let (audio, audio_duration_s) = if self.st.audio.trim().is_empty() {
            (None, None)
        } else {
            let a = check_input(&self.st.audio, AUDIO_EXTS, "audio file")?;
            let info = self.audio_probe.require("Audio file")?;
            let d = info.any_duration();
            (Some(a), d)
        };
        if self.st.duration_mode == DurationMode::MatchAudio && audio.is_none() {
            return Err("Match audio length needs an audio file.".into());
        }
        let has_xfade = cx.has_xfade.unwrap_or(false);
        let pr = SlideshowParams {
            images,
            audio,
            audio_duration_s,
            output: output.clone(),
            duration_mode: self.st.duration_mode,
            per_image_s: self.st.per_image_s,
            fixed_total_s: self.st.fixed_total_s,
            transition: self.st.transition,
            fade_s: self.st.fade_s,
            has_xfade,
            size: self.st.size,
            small: self.st.small,
            fps: self.st.fps,
            video: self.st.video.clone(),
            audio_bitrate_k: self.st.audio_bitrate_k,
            faststart: self.st.faststart,
        };
        let t = slideshow::timing(&pr)?;
        let cmd = slideshow::build(&pr)?;
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Slideshow -> {}", self.st.out_name), cmd, output.clone(), log_path_for(&output), Some(t.total_s))],
            ..Default::default()
        })
    }

    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        self.add_paths(paths.iter().cloned());
        if let Some(p) = first_with_ext(paths, AUDIO_EXTS) {
            self.st.audio = s(p);
        }
    }

    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State::from_settings(s));
        self.dims.clear();
        self.thumbs.clear();
        self.audio_probe = ProbeSlot::default();
        self.drag_from = None;
    }
}

impl SlideshowMode {
    fn params_preview(&self, images: Vec<ImageInput>, cx: &Ctx) -> SlideshowParams {
        SlideshowParams {
            images,
            audio: (!self.st.audio.trim().is_empty()).then(|| PathBuf::from(self.st.audio.trim())),
            audio_duration_s: self.audio_probe.info().and_then(|i| i.any_duration()),
            output: PathBuf::from("preview.mp4"),
            duration_mode: self.st.duration_mode,
            per_image_s: self.st.per_image_s,
            fixed_total_s: self.st.fixed_total_s,
            transition: self.st.transition,
            fade_s: self.st.fade_s,
            has_xfade: cx.has_xfade.unwrap_or(false),
            size: self.st.size,
            small: self.st.small,
            fps: self.st.fps,
            video: self.st.video.clone(),
            audio_bitrate_k: self.st.audio_bitrate_k,
            faststart: self.st.faststart,
        }
    }
}
