//! Crop section and watermark placement (1.1), shared by every preset that re-encodes video.
//!
//! * [`Grabber`] – cached, background frame grabs (the app's FFmpeg) turned into textures.
//! * [`crop_section`] – "Crop" section: Enable box, Top/Right/Bottom/Left in clock positions,
//!   checks with a "Round to even" fix, 4 thumbnails (20/40/60/80 %) with the crop drawn on them.
//! * [`watermark_row`] / [`watermark_window`] – "Add watermark" checkbox + "Set up watermark…"
//!   wizard for the encode presets; [`watermark_editor`] is the same editor, also used inline by
//!   the Add Watermark preset.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};
use ffenc::cmd::picture::{CropSettings, PictureSettings, Severity, Snap, WatermarkSettings};
use ffenc::ffmpeg::framegrab::{self, Frame};
use ffenc::ffmpeg::probe::MediaInfo;

use super::common::{hint, hover_tip, path_row, row_label_tip, Bg, Pick};
use super::modes::Ctx;
use super::theme;

/// Watermark images offered by Browse (any file FFmpeg reads also works via "All files").
pub const WM_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"];
/// Where the 4 crop thumbnails are taken.
pub const THUMB_AT: [f64; 4] = [0.2, 0.4, 0.6, 0.8];
/// Where the watermark preview frame is taken.
pub const PREVIEW_AT: f64 = 0.5;

pub const CROP_TIP: &str = "Cut pixels off the edges of the picture, for example black bars or a game HUD. One crop is used for every file. Off by default.";
pub const WM_TIP: &str = "Put an image (a logo, a name) on top of the video. PNG transparency is kept. Off by default.";

/// One video the previews can show.
#[derive(Debug, Clone, PartialEq)]
pub struct PicSource {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}

impl PicSource {
    pub fn from_info(path: &Path, i: &MediaInfo) -> Option<Self> {
        let v = i.video()?;
        Some(PicSource {
            path: path.to_path_buf(),
            width: v.width.unwrap_or(0),
            height: v.height.unwrap_or(0),
            duration: i.duration().or_else(|| i.any_duration()).unwrap_or(0.0),
        })
    }
    pub fn name(&self) -> String {
        self.path.file_name().unwrap_or_default().to_string_lossy().into_owned()
    }
}

/// The FFmpeg the app is using (same folder as FFprobe when only that is known).
pub fn ffmpeg_path(cx: &Ctx) -> Option<PathBuf> {
    if let Some(f) = cx.ff {
        return Some(f.ffmpeg.clone());
    }
    cx.ffprobe.map(|fp| {
        let ext = fp.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        fp.with_file_name(format!("ffmpeg{ext}"))
    })
}

// ---------------------------------------------------------------- frame cache

enum Slot {
    Busy(Bg<Result<(Frame, (u32, u32)), String>>),
    Ready(TextureHandle, (u32, u32)),
    Failed(String),
}

pub enum Got<'a> {
    Busy,
    Ready(&'a TextureHandle),
    Failed(&'a str),
}

/// Cached frame grabs. Keys: (file, position in ‰ or u32::MAX for still images, width).
#[derive(Default)]
pub struct Grabber {
    slots: HashMap<(PathBuf, u32, u32), Slot>,
}

const MAX_SLOTS: usize = 48;
const MAX_IMAGE_SIDE: u32 = 1024;

impl Grabber {
    fn poll(&mut self, ctx: &egui::Context) {
        for (k, slot) in self.slots.iter_mut() {
            if let Slot::Busy(t) = slot {
                if let Some(r) = t.poll() {
                    *slot = match r {
                        Ok((f, orig)) => {
                            let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                            Slot::Ready(ctx.load_texture(format!("grab_{}_{}_{}", k.0.display(), k.1, k.2), img, egui::TextureOptions::LINEAR), orig)
                        }
                        Err(e) => Slot::Failed(e),
                    };
                }
            }
        }
    }

    fn request(&mut self, ctx: &egui::Context, ffmpeg: Option<&Path>, path: &Path, key: u32, at: Option<f64>, width: Option<u32>) -> Got<'_> {
        self.poll(ctx);
        let k = (path.to_path_buf(), key, width.unwrap_or(0));
        if !self.slots.contains_key(&k) {
            if self.slots.len() >= MAX_SLOTS {
                self.slots.retain(|_, s| matches!(s, Slot::Busy(_)));
            }
            let slot = match ffmpeg {
                None => Slot::Failed("FFmpeg not found – set the FFmpeg folder in Settings.".into()),
                Some(_) if !path.is_file() => Slot::Failed("File not found.".into()),
                Some(ff) => {
                    let (ff, p) = (ff.to_path_buf(), path.to_path_buf());
                    Slot::Busy(Bg::spawn(ctx, move || {
                        let f = framegrab::grab(&ff, &p, at, width)?;
                        let orig = (f.width, f.height);
                        Ok((shrink(f), orig))
                    }))
                }
            };
            self.slots.insert(k.clone(), slot);
        }
        match &self.slots[&k] {
            Slot::Busy(_) => Got::Busy,
            Slot::Ready(t, _) => Got::Ready(t),
            Slot::Failed(e) => Got::Failed(e),
        }
    }

    /// A frame `frac` (0–1) through `src`, `width` pixels wide.
    pub fn frame(&mut self, ctx: &egui::Context, ffmpeg: Option<&Path>, src: &PicSource, frac: f64, width: u32) -> Got<'_> {
        let w = if src.width > 0 { width.min(src.width) } else { width };
        self.request(ctx, ffmpeg, &src.path, (frac * 1000.0).round() as u32, Some(framegrab::time_at(src.duration, frac)), Some(w))
    }

    /// A still image (the watermark), with its own size.
    pub fn image(&mut self, ctx: &egui::Context, ffmpeg: Option<&Path>, path: &Path) -> Got<'_> {
        self.request(ctx, ffmpeg, path, u32::MAX, None, None)
    }

    /// Pixel size of a loaded still image.
    pub fn image_size(&self, path: &Path) -> Option<(u32, u32)> {
        match self.slots.get(&(path.to_path_buf(), u32::MAX, 0)) {
            Some(Slot::Ready(_, s)) => Some(*s),
            _ => None,
        }
    }
}

/// Keep textures small (big logos would waste memory).
fn shrink(f: Frame) -> Frame {
    if f.width <= MAX_IMAGE_SIDE && f.height <= MAX_IMAGE_SIDE {
        return f;
    }
    let Some(img) = image::RgbaImage::from_raw(f.width, f.height, f.rgba.clone()) else { return f };
    let s = MAX_IMAGE_SIDE as f64 / f.width.max(f.height) as f64;
    let (w, h) = (((f.width as f64 * s).round() as u32).max(1), ((f.height as f64 * s).round() as u32).max(1));
    let r = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    Frame { width: w, height: h, rgba: r.into_raw() }
}

// ---------------------------------------------------------------- runtime state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drag {
    Move,
    /// Corner: (moves left edge, moves top edge)
    Corner(bool, bool),
}

/// Per-form state that is not saved (selected file, open wizard, drags, frame cache).
#[derive(Default)]
pub struct PictureRt {
    pub grab: Grabber,
    pub sel: usize,
    pub wizard_open: bool,
    drag: Option<(Drag, WatermarkSettings, Pos2)>,
}

impl PictureRt {
    /// Watermark image size, once it has been read.
    pub fn image_size(&self, wm: &WatermarkSettings) -> Option<(u32, u32)> {
        let p = wm.image.trim();
        (!p.is_empty()).then(|| self.grab.image_size(Path::new(p))).flatten()
    }
}

fn file_picker(ui: &mut Ui, id: &str, srcs: &[PicSource], sel: &mut usize) {
    if *sel >= srcs.len() {
        *sel = 0;
    }
    if srcs.len() > 1 {
        ui.horizontal(|ui| {
            ui.label("Show file");
            egui::ComboBox::from_id_salt((id, "pic_file")).width(320.0).selected_text(srcs[*sel].name()).show_ui(ui, |ui| {
                for (i, s) in srcs.iter().enumerate() {
                    ui.selectable_value(sel, i, format!("{}  ({}×{})", s.name(), s.width, s.height));
                }
            });
        });
    }
}

fn status_box(ui: &mut Ui, size: Vec2, text: &str, busy: bool) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(r, 4.0, Color32::from_gray(40));
    ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0_f32, Color32::from_gray(110)), StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(8.0)).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)));
    if busy {
        child.spinner();
    } else {
        child.label(RichText::new(text).color(Color32::from_gray(230)));
    }
}

// ---------------------------------------------------------------- crop

fn crop_box(ui: &mut Ui, label: &str, v: &mut u32, max: u32) -> bool {
    ui.label(RichText::new(label).strong());
    let r = ui.add(egui::DragValue::new(v).range(0..=max.max(1)).speed(1.0).suffix(" px"));
    hover_tip(r, format!("Pixels to cut off the {} edge. Use even numbers.", label.to_lowercase())).changed()
}

/// Crop rectangle drawn over a picture of a `sw`×`sh` source shown in `r`.
fn paint_crop(p: &egui::Painter, r: Rect, sw: u32, sh: u32, c: &CropSettings) {
    if sw == 0 || sh == 0 || !c.active() {
        return;
    }
    let (sx, sy) = (r.width() / sw as f32, r.height() / sh as f32);
    let inner = Rect::from_min_max(
        egui::pos2(r.left() + c.left as f32 * sx, r.top() + c.top as f32 * sy),
        egui::pos2(r.right() - c.right as f32 * sx, r.bottom() - c.bottom as f32 * sy),
    );
    let inner = Rect::from_min_max(inner.min.min(r.max), inner.max.max(inner.min.min(r.max)).min(r.max));
    let dim = Color32::from_black_alpha(170);
    p.rect_filled(Rect::from_min_max(r.min, egui::pos2(r.right(), inner.top())), 0.0, dim);
    p.rect_filled(Rect::from_min_max(egui::pos2(r.left(), inner.bottom()), r.max), 0.0, dim);
    p.rect_filled(Rect::from_min_max(egui::pos2(r.left(), inner.top()), egui::pos2(inner.left(), inner.bottom())), 0.0, dim);
    p.rect_filled(Rect::from_min_max(egui::pos2(inner.right(), inner.top()), egui::pos2(r.right(), inner.bottom())), 0.0, dim);
    p.rect_stroke(inner, 0.0, Stroke::new(3.0_f32, Color32::BLACK), StrokeKind::Middle);
    p.rect_stroke(inner, 0.0, Stroke::new(2.0_f32, Color32::from_rgb(255, 215, 0)), StrokeKind::Middle);
}

/// Plain text for one file's crop checks (worst first).
pub fn crop_issues(c: &CropSettings, srcs: &[PicSource]) -> Vec<(Severity, String)> {
    let mut sizes: Vec<(u32, u32)> = srcs.iter().filter(|s| s.width > 0 && s.height > 0).map(|s| (s.width, s.height)).collect();
    sizes.dedup();
    let multi = {
        let mut u = sizes.clone();
        u.sort();
        u.dedup();
        u.len() > 1
    };
    let mut out = vec![];
    for s in srcs.iter().filter(|s| s.width > 0) {
        for i in c.check(s.width, s.height) {
            let t = if multi { format!("{} ({}×{}): {}", s.name(), s.width, s.height, i.text) } else { i.text };
            if !out.iter().any(|(_, x)| *x == t) {
                out.push((i.severity, t));
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// The "Crop" section. `srcs` = the form's probed videos (may be empty).
pub fn crop_section(ui: &mut Ui, cx: &Ctx, id: &str, crop: &mut CropSettings, srcs: &[PicSource], rt: &mut PictureRt) {
    crate::ui::layout::section(ui, &format!("{id}.crop"), egui::CollapsingHeader::new(RichText::new("Crop").strong()), crop.enabled, |ui| {
        let r = ui.checkbox(&mut crop.enabled, "Enable cropping");
        hover_tip(r, CROP_TIP);
        if !crop.enabled {
            hint(ui, "Off: the picture is not cropped.");
            return;
        }
        file_picker(ui, id, srcs, &mut rt.sel);
        let cur = srcs.get(rt.sel);
        let (sw, sh) = cur.map(|s| (s.width, s.height)).unwrap_or((0, 0));
        let maxw = if sw > 0 { sw } else { 8192 };
        let maxh = if sh > 0 { sh } else { 8192 };
        // clock layout: Top at 12, Right at 3, Bottom at 6, Left at 9
        let cell = egui::vec2(150.0, ui.text_style_height(&egui::TextStyle::Body) * 2.0 + ui.spacing().interact_size.y + 8.0);
        let mid = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui)| {
            ui.allocate_ui_with_layout(cell, egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_min_size(cell);
                add(ui)
            });
        };
        ui.horizontal(|ui| {
            mid(ui, &mut |_| {});
            mid(ui, &mut |ui| {
                crop_box(ui, "Top", &mut crop.top, maxh);
            });
        });
        ui.horizontal(|ui| {
            mid(ui, &mut |ui| {
                crop_box(ui, "Left", &mut crop.left, maxw);
            });
            mid(ui, &mut |ui| {
                if sw > 0 {
                    ui.label(format!("Source {sw}×{sh}"));
                    match crop.result_size(sw, sh) {
                        Some((w, h)) => ui.label(RichText::new(format!("Result {w}×{h}")).strong()),
                        None => ui.colored_label(theme::c().err, "Nothing left"),
                    };
                } else {
                    hint(ui, "Choose a video to see its size.");
                }
            });
            mid(ui, &mut |ui| {
                crop_box(ui, "Right", &mut crop.right, maxw);
            });
        });
        ui.horizontal(|ui| {
            mid(ui, &mut |_| {});
            mid(ui, &mut |ui| {
                crop_box(ui, "Bottom", &mut crop.bottom, maxh);
            });
        });
        let issues = crop_issues(crop, srcs);
        let mut fix = false;
        for (sev, t) in &issues {
            match sev {
                Severity::Error => {
                    ui.label(RichText::new(format!("✖ {t}")).color(theme::c().err).strong());
                }
                Severity::Warn => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("⚠ {t}")).color(theme::c().err));
                    });
                }
                Severity::Note => hint(ui, t.clone()),
            }
        }
        if issues.iter().any(|(s, _)| *s == Severity::Warn) {
            fix = hover_tip(ui.button("Round to even"), "Change odd values to even numbers so normal video encodes cleanly.").clicked();
        }
        if fix {
            crop.round_to_even(sw, sh);
        }
        if let Some(src) = cur {
            let ff = ffmpeg_path(cx);
            let avail = ui.available_width().max(300.0);
            let tw = ((avail - 3.0 * 10.0) / 4.0).clamp(160.0, 300.0);
            let th = if src.width > 0 { tw * src.height as f32 / src.width as f32 } else { tw * 9.0 / 16.0 };
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for f in THUMB_AT {
                    ui.vertical(|ui| {
                        match rt.grab.frame(cx.egui, ff.as_deref(), src, f, 480) {
                            Got::Ready(t) => {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(tw, th), Sense::hover());
                                ui.painter().image(t.id(), r, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                                paint_crop(ui.painter(), r, src.width, src.height, crop);
                            }
                            Got::Busy => status_box(ui, egui::vec2(tw, th), "", true),
                            Got::Failed(e) => status_box(ui, egui::vec2(tw, th), e, false),
                        }
                        hint(ui, format!("{} %", (f * 100.0) as u32));
                    });
                }
            });
        } else {
            hint(ui, "Choose a video to see thumbnails.");
        }
    });
}

// ---------------------------------------------------------------- watermark

/// Grid row: "Watermark  [x] Add watermark  [Set up watermark…]  logo.png".
pub fn watermark_row(ui: &mut Ui, wm: &mut WatermarkSettings, rt: &mut PictureRt) {
    row_label_tip(ui, "Watermark", WM_TIP);
    ui.horizontal_wrapped(|ui| {
        let was = wm.enabled;
        hover_tip(ui.checkbox(&mut wm.enabled, "Add watermark"), WM_TIP);
        if wm.enabled && !was && wm.image.trim().is_empty() {
            rt.wizard_open = true; // walk the user through it
        }
        if wm.enabled {
            if ui.button("Set up watermark…").clicked() {
                rt.wizard_open = true;
            }
            let img = wm.image.trim();
            if img.is_empty() {
                ui.colored_label(theme::c().err, "No image chosen yet");
            } else {
                let name = Path::new(img).file_name().unwrap_or_default().to_string_lossy().into_owned();
                ui.label(format!("{name} · {} % opacity", wm.opacity_pct));
            }
        }
    });
    ui.end_row();
}

/// The wizard window for encode presets (open while `rt.wizard_open`).
pub fn watermark_window(cx: &Ctx, id: &str, pic: &mut PictureSettings, srcs: &[PicSource], rt: &mut PictureRt) {
    if !rt.wizard_open || !pic.watermark.enabled {
        rt.wizard_open = false;
        return;
    }
    let mut open = true;
    let mut done = false;
    super::common::fit(egui::Window::new("Set up watermark").id(egui::Id::new((id, "wm_wizard"))).open(&mut open).collapsible(false).resizable(true).default_width(820.0), cx.egui, true)
        .remember("watermark", true)
        .show(cx.egui, |ui| {
            watermark_editor(ui, cx, id, pic, srcs, rt);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Done").strong())).clicked() {
                    done = true;
                }
                hint(ui, "Settings are kept with this job and saved with the preset.");
            });
        });
    if !open || done {
        rt.wizard_open = false;
    }
}

/// Frame (cropped) size for placing the watermark on `src`.
fn frame_size(pic: &PictureSettings, src: Option<&PicSource>) -> (u32, u32) {
    match src {
        Some(s) if s.width > 0 && s.height > 0 => {
            if pic.crop.active() {
                pic.crop.result_size(s.width, s.height).unwrap_or((s.width, s.height))
            } else {
                (s.width, s.height)
            }
        }
        _ => (1920, 1080),
    }
}

/// The watermark editor: image, preview with drag + resize handles, numbers, snaps, opacity.
pub fn watermark_editor(ui: &mut Ui, cx: &Ctx, id: &str, pic: &mut PictureSettings, srcs: &[PicSource], rt: &mut PictureRt) {
    let ff = ffmpeg_path(cx);
    egui::Grid::new((id, "wm_img")).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        path_row(ui, "Watermark image", &mut pic.watermark.image, Pick::File("Images", WM_IMAGE_EXTS), &cx.settings.input_folder);
    });
    let img_path = PathBuf::from(pic.watermark.image.trim());
    let mut img_tex: Option<TextureHandle> = None;
    let mut img_err: Option<String> = None;
    if !pic.watermark.image.trim().is_empty() {
        match rt.grab.image(cx.egui, ff.as_deref(), &img_path) {
            Got::Ready(t) => img_tex = Some(t.clone()),
            Got::Busy => {}
            Got::Failed(e) => img_err = Some(e.to_string()),
        }
    }
    let img_size = rt.image_size(&pic.watermark);
    if let Some(e) = &img_err {
        ui.colored_label(theme::c().err, format!("Can't read the image. {e}"));
    } else if let Some((w, h)) = img_size {
        hint(ui, format!("Image {w}×{h}"));
    } else if pic.watermark.image.trim().is_empty() {
        ui.colored_label(theme::c().warn, "Choose an image: a PNG with transparency works best.");
    }

    file_picker(ui, &format!("{id}_wm"), srcs, &mut rt.sel);
    let src = srcs.get(rt.sel);
    let (fw, fh) = frame_size(pic, src);
    let (fwf, fhf) = (fw as f64, fh as f64);

    // ---- preview canvas
    let avail = ui.available_width().clamp(320.0, 960.0);
    let mut cw = avail;
    let mut chh = cw * fh as f32 / fw as f32;
    if chh > 540.0 {
        chh = 540.0;
        cw = chh * fw as f32 / fh as f32;
    }
    let (resp, painter) = ui.allocate_painter(egui::vec2(cw, chh), Sense::click_and_drag());
    let r = resp.rect;
    let frame_tex = src.map(|s| rt.grab.frame(cx.egui, ff.as_deref(), s, PREVIEW_AT, 960));
    match frame_tex {
        Some(Got::Ready(t)) => {
            let s = src.unwrap();
            let c = &pic.crop;
            let uv = if c.active() && s.width > 0 && s.height > 0 && c.result_size(s.width, s.height).is_some() {
                Rect::from_min_max(
                    egui::pos2(c.left as f32 / s.width as f32, c.top as f32 / s.height as f32),
                    egui::pos2(1.0 - c.right as f32 / s.width as f32, 1.0 - c.bottom as f32 / s.height as f32),
                )
            } else {
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0))
            };
            painter.image(t.id(), r, uv, Color32::WHITE);
        }
        other => {
            painter.rect_filled(r, 0.0, Color32::from_gray(45));
            let msg = match other {
                Some(Got::Busy) => "Loading the preview frame…".to_string(),
                Some(Got::Failed(e)) => e.to_string(),
                _ => "Choose a video to see a frame from the middle of it.".to_string(),
            };
            painter.text(r.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(16.0), Color32::from_gray(225));
        }
    }
    painter.rect_stroke(r, 0.0, Stroke::new(1.0_f32, Color32::from_gray(120)), StrokeKind::Outside);

    let scale = r.width() as f64 / fwf;
    let wm = &mut pic.watermark;
    let (wpx, hpx) = wm.size_px(fwf, fhf, img_size);
    let wr = Rect::from_min_size(
        egui::pos2(r.left() + (wm.x * fwf * scale) as f32, r.top() + (wm.y * fhf * scale) as f32),
        egui::vec2((wpx * scale) as f32, (hpx * scale) as f32),
    );
    let clip = painter.with_clip_rect(r);
    if let Some(t) = &img_tex {
        let a = (wm.opacity() * 255.0).round() as u8;
        clip.image(t.id(), wr, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::from_white_alpha(a));
    } else {
        clip.rect_filled(wr, 0.0, Color32::from_white_alpha(60));
        clip.text(wr.center(), egui::Align2::CENTER_CENTER, "Watermark", egui::FontId::proportional(14.0), Color32::WHITE);
    }
    clip.rect_stroke(wr, 0.0, Stroke::new(3.0_f32, Color32::BLACK), StrokeKind::Middle);
    clip.rect_stroke(wr, 0.0, Stroke::new(1.5_f32, Color32::from_rgb(255, 215, 0)), StrokeKind::Middle);
    let hs = 16.0;
    let corners = [(wr.left_top(), true, true), (wr.right_top(), false, true), (wr.left_bottom(), true, false), (wr.right_bottom(), false, false)];
    for (p, _, _) in corners {
        let h = Rect::from_center_size(p, egui::vec2(hs, hs));
        painter.rect_filled(h, 2.0, Color32::from_rgb(255, 215, 0));
        painter.rect_stroke(h, 2.0, Stroke::new(2.0_f32, Color32::BLACK), StrokeKind::Inside);
    }

    // ---- mouse: drag to move, corner handles resize
    let hit = |pos: Pos2| -> Option<Drag> {
        for (p, l, t) in corners {
            if Rect::from_center_size(p, egui::vec2(hs + 8.0, hs + 8.0)).contains(pos) {
                return Some(Drag::Corner(l, t));
            }
        }
        wr.contains(pos).then_some(Drag::Move)
    };
    if let Some(pos) = resp.hover_pos() {
        match rt.drag.as_ref().map(|d| d.0).or_else(|| hit(pos)) {
            Some(Drag::Move) => cx.egui.set_cursor_icon(egui::CursorIcon::Grab),
            Some(Drag::Corner(l, t)) => cx.egui.set_cursor_icon(if l == t { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw }),
            None => {}
        }
    }
    if resp.drag_started() {
        if let Some(pos) = resp.interact_pointer_pos() {
            rt.drag = hit(pos).map(|d| (d, wm.clone(), pos));
        }
    }
    if let (Some((d, start, p0)), Some(pos)) = (rt.drag.clone(), resp.interact_pointer_pos()) {
        if resp.dragged() {
            let dx = (pos.x - p0.x) as f64 / scale;
            let dy = (pos.y - p0.y) as f64 / scale;
            let (sw, sh) = start.size_px(fwf, fhf, img_size);
            let (sx, sy) = (start.x * fwf, start.y * fhf);
            match d {
                Drag::Move => {
                    wm.x = ((sx + dx) / fwf).clamp(0.0, (1.0 - sw / fwf).max(0.0));
                    wm.y = ((sy + dy) / fhf).clamp(0.0, (1.0 - sh / fhf).max(0.0));
                }
                Drag::Corner(left, top) => {
                    let min = 8.0;
                    let mut nw = (if left { sw - dx } else { sw + dx }).max(min);
                    let mut nh = (if top { sh - dy } else { sh + dy }).max(min);
                    if wm.lock_aspect {
                        let aspect = sh / sw;
                        // follow whichever edge moved more
                        if (nw - sw).abs() * aspect >= (nh - sh).abs() {
                            nh = nw * aspect;
                        } else {
                            nw = nh / aspect;
                        }
                    }
                    let nx = if left { sx + sw - nw } else { sx };
                    let ny = if top { sy + sh - nh } else { sy };
                    wm.w = nw / fwf;
                    wm.h = nh / fhf;
                    wm.x = (nx / fwf).max(0.0);
                    wm.y = (ny / fhf).max(0.0);
                }
            }
        }
    }
    if resp.drag_stopped() || !resp.dragged() && !resp.drag_started() {
        rt.drag = None;
    }
    hint(ui, "Drag the image to move it. Drag a yellow corner to resize it.");

    // ---- precise numbers (pixels of this file's picture)
    ui.add_space(4.0);
    let (wpx, hpx) = wm.size_px(fwf, fhf, img_size);
    let (mut x, mut y, mut w, mut h) = ((wm.x * fwf).round(), (wm.y * fhf).round(), wpx.round(), hpx.round());
    egui::Grid::new((id, "wm_nums")).num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        row_label_tip(ui, "Position", "Left and top edge of the watermark, in pixels of this file's picture (after cropping). Saved as a share of the frame, so other sizes match.");
        ui.horizontal(|ui| {
            ui.label("X");
            if ui.add(egui::DragValue::new(&mut x).range(0.0..=fwf).speed(1.0).suffix(" px")).changed() {
                wm.x = x / fwf;
            }
            ui.label("Y");
            if ui.add(egui::DragValue::new(&mut y).range(0.0..=fhf).speed(1.0).suffix(" px")).changed() {
                wm.y = y / fhf;
            }
        });
        ui.end_row();
        row_label_tip(ui, "Size", "Width and height in pixels. With “Keep the image's shape” on, changing one changes the other.");
        ui.horizontal(|ui| {
            ui.label("Width");
            let wc = ui.add(egui::DragValue::new(&mut w).range(1.0..=fwf * 2.0).speed(1.0).suffix(" px")).changed();
            ui.label("Height");
            let hc = ui.add(egui::DragValue::new(&mut h).range(1.0..=fhf * 2.0).speed(1.0).suffix(" px")).changed();
            if wc {
                wm.w = w / fwf;
                if !wm.lock_aspect {
                    wm.h = h / fhf;
                }
            } else if hc {
                if wm.lock_aspect && hpx > 0.0 {
                    wm.w = h * wpx / hpx / fwf;
                } else {
                    wm.h = h / fhf;
                }
            }
            let r = ui.checkbox(&mut wm.lock_aspect, "Keep the image's shape");
            if r.changed() && !wm.lock_aspect {
                wm.h = hpx / fhf; // start from the shape it has now
            }
            hover_tip(r, "Locks the aspect ratio so the image is never stretched. Untick to set width and height freely.");
        });
        ui.end_row();
        row_label_tip(ui, "Snap to", "Move the watermark to a corner or the center, keeping the margin from the edges.");
        ui.horizontal_wrapped(|ui| {
            for s in Snap::ALL {
                if ui.button(s.label()).clicked() {
                    wm.snap(s, fwf, fhf, img_size);
                }
            }
        });
        ui.end_row();
        row_label_tip(ui, "Margin", "Gap the snap buttons keep from the edges, as a share of the picture's shorter side.");
        ui.add(egui::DragValue::new(&mut wm.margin_pct).range(0.0..=25.0).speed(0.1).max_decimals(1).suffix(" %"));
        ui.end_row();
        row_label_tip(ui, "Opacity", "How solid the watermark is: 100 % = fully visible, lower = see-through.");
        ui.add(egui::Slider::new(&mut wm.opacity_pct, 0..=100).suffix(" %"));
        ui.end_row();
    });
    if let Ok((_, warns)) = wm.resolve(fw, fh, img_size) {
        for w in warns {
            ui.colored_label(theme::c().warn, format!("⚠ {w}"));
        }
    }
}

/// Resolve both options for a form's build step (single file); warnings go to the preview.
pub fn resolve_for(pic: &PictureSettings, rt: &PictureRt, w: u32, h: u32) -> Result<(ffenc::cmd::picture::PictureOps, Vec<String>), String> {
    if !pic.any() {
        return Ok(Default::default());
    }
    if pic.watermark.enabled && !pic.watermark.image.trim().is_empty() && !Path::new(pic.watermark.image.trim()).is_file() {
        return Err(format!("Watermark image not found: {}", pic.watermark.image.trim()));
    }
    pic.resolve(w, h, rt.image_size(&pic.watermark))
}

/// Checks before building a multi-file job (image exists).
pub fn check_image(pic: &PictureSettings) -> Result<(), String> {
    let p = pic.watermark.image.trim();
    if pic.watermark.enabled && !p.is_empty() && !Path::new(p).is_file() {
        return Err(format!("Watermark image not found: {p}"));
    }
    Ok(())
}
