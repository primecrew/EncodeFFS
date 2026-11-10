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
use ffenc::settings::{Settings, SlideshowThumbSize, SlideshowUi};
use ffenc::util::{fmt_hms, fmt_num, natural_cmp, natural_sort_paths, stem, AUDIO_EXTS};

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
    decode_side: u32,
}

pub struct SlideshowMode {
    st: State,
    dims: HashMap<PathBuf, DimProbe>,
    thumbs: HashMap<PathBuf, Thumb>,
    audio_probe: ProbeSlot,
    /// Selected row in the image list (keyboard / click).
    selected: Option<usize>,
    /// List display prefs (filenames, thumbs, pop-out). Independent of the preset.
    list_ui: SlideshowUi,
    /// Apply saved pop-out size the next time the viewport is opened.
    popout_apply_size: bool,
    /// Skip rewriting slideshow-ui.json when nothing changed.
    list_ui_saved: SlideshowUi,
}

impl Default for SlideshowMode {
    fn default() -> Self {
        let list_ui = SlideshowUi::load();
        let popout_apply_size = list_ui.list_popped_out;
        SlideshowMode {
            st: State::default(),
            dims: HashMap::new(),
            thumbs: HashMap::new(),
            audio_probe: ProbeSlot::default(),
            selected: None,
            list_ui: list_ui.clone(),
            popout_apply_size,
            list_ui_saved: list_ui,
        }
    }
}

fn modified(p: &Path) -> SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

fn load_thumb(ctx: &egui::Context, path: &Path, decode_side: u32) -> Option<Thumb> {
    let mtime = modified(path);
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = img.thumbnail(decode_side, decode_side);
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    let tex = ctx.load_texture(format!("ss_{}_{}", path.display(), decode_side), color, TextureOptions::LINEAR);
    Some(Thumb { tex, mtime, decode_side })
}

fn drag_handle(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(14.0, 22.0), egui::Sense::drag());
    let c = if resp.hovered() || resp.dragged() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    for i in 0..3 {
        for j in 0..2 {
            ui.painter().circle_filled(egui::pos2(rect.left() + 4.0 + j as f32 * 5.5, rect.center().y - 6.0 + i as f32 * 6.0), 1.6, c);
        }
    }
    named(resp, egui::WidgetType::DragValue, "Drag handle: drag to reorder (or use the Move up / Move down buttons)")
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
        self.dims.retain(|p, _| self.st.images.iter().any(|s| Path::new(s) == p));
        self.thumbs.retain(|p, _| self.st.images.iter().any(|s| Path::new(s) == p));
        if let Some(s) = self.selected {
            if s >= self.st.images.len() {
                self.selected = self.st.images.len().checked_sub(1);
            }
        }
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

    fn persist_list_ui(&mut self, force: bool) {
        self.list_ui = self.list_ui.clone().sanitized();
        let only_size = self.list_ui.show_filenames == self.list_ui_saved.show_filenames
            && self.list_ui.show_thumbnails == self.list_ui_saved.show_thumbnails
            && self.list_ui.thumb_size == self.list_ui_saved.thumb_size
            && self.list_ui.list_popped_out == self.list_ui_saved.list_popped_out
            && (self.list_ui.popout_w != self.list_ui_saved.popout_w || self.list_ui.popout_h != self.list_ui_saved.popout_h);
        if self.list_ui != self.list_ui_saved && (force || !only_size) {
            if let Err(e) = self.list_ui.save() {
                eprintln!("Could not save slideshow list settings: {e}");
            }
            self.list_ui_saved = self.list_ui.clone();
        }
    }

    fn per_image_secs(&self, cx: &Ctx) -> Option<f64> {
        let images = self.image_inputs().ok()?;
        slideshow::timing(&self.params_preview(images, cx)).ok().map(|t| t.per_image_s)
    }

    fn apply_move(&mut self, from: usize, before: usize) {
        let n = self.st.images.len();
        self.selected = slideshow::map_selection_after_move(self.selected, from, before, n);
        if slideshow::move_before(&mut self.st.images, from, before).is_some() {
            self.update_name();
        }
    }

    fn apply_remove(&mut self, i: usize) {
        if i < self.st.images.len() {
            self.st.images.remove(i);
            self.selected = slideshow::map_selection_after_remove(self.selected, i, self.st.images.len());
            self.update_name();
        }
    }

    fn sort_keep_sel(&mut self, f: impl FnOnce(&mut Vec<String>)) {
        let cur = self.selected.and_then(|i| self.st.images.get(i).cloned());
        f(&mut self.st.images);
        if let Some(c) = cur {
            self.selected = self.st.images.iter().position(|x| x == &c);
        }
    }

    fn list_toolbar(&mut self, ui: &mut egui::Ui, cx: &Ctx, in_popout: bool) {
        let n = self.st.images.len();
        ui.horizontal_wrapped(|ui| {
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
                self.sort_keep_sel(|v| v.sort_by(|a, b| natural_cmp(&stem(Path::new(a)), &stem(Path::new(b))).then_with(|| natural_cmp(a, b))));
            }
            if ui.add_enabled(n > 0, egui::Button::new("Sort by modified date")).on_hover_text("Oldest first").clicked() {
                self.sort_keep_sel(|v| v.sort_by_key(|a| modified(Path::new(a))));
            }
            if ui.add_enabled(n > 1, egui::Button::new("Reverse")).clicked() {
                self.sort_keep_sel(|v| v.reverse());
            }
            if ui.add_enabled(n > 0, egui::Button::new("Clear")).clicked() {
                self.st.images.clear();
                self.selected = None;
                self.update_name();
            }
            ui.label(RichText::new(format!("{n} image{}", if n == 1 { "" } else { "s" })).weak());
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.list_ui.show_filenames, "Show filenames")
                .on_hover_text("Show or hide each image's file name in the list. The name stays in the accessible label for screen readers.");
            ui.checkbox(&mut self.list_ui.show_thumbnails, "Show thumbnails").on_hover_text("Show or hide the preview of each image.");
            if self.list_ui.show_thumbnails {
                ui.label("Thumbnail size");
                let salt = if in_popout { "ss_thumb_pop" } else { "ss_thumb" };
                egui::ComboBox::from_id_salt(salt).width(100.0).selected_text(self.list_ui.thumb_size.label()).show_ui(ui, |ui| {
                    for sz in SlideshowThumbSize::ALL {
                        ui.selectable_value(&mut self.list_ui.thumb_size, sz, sz.label());
                    }
                });
            }
            if self.list_ui.list_popped_out {
                if named(ui.button("Pop in"), egui::WidgetType::Button, "Pop in: put the image list back in the main window").clicked() {
                    self.list_ui.list_popped_out = false;
                    self.persist_list_ui(true);
                }
            } else if named(ui.button("Pop out"), egui::WidgetType::Button, "Pop out: open the image list in a separate resizable window").clicked() {
                self.list_ui.list_popped_out = true;
                self.popout_apply_size = true;
                self.persist_list_ui(true);
            }
        });
    }

    fn handle_list_keys(&mut self, ui: &mut egui::Ui, list_id: egui::Id) {
        let n = self.st.images.len();
        if n == 0 || !ui.memory(|m| m.has_focus(list_id)) {
            return;
        }
        let mods = ui.input(|i| i.modifiers);
        let move_sel = |sel: &mut Option<usize>, dir: i32| {
            if n == 0 {
                return;
            }
            let cur = sel.unwrap_or(if dir < 0 { 0 } else { n - 1 });
            let next = if dir < 0 { cur.saturating_sub(1) } else { (cur + 1).min(n - 1) };
            *sel = Some(next);
        };
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
            move_sel(&mut self.selected, -1);
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
            move_sel(&mut self.selected, 1);
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Home)) {
            self.selected = Some(0);
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::End)) {
            self.selected = Some(n - 1);
        } else if (mods.alt || mods.ctrl)
            && ui.input_mut(|i| i.consume_key(mods, egui::Key::ArrowUp))
        {
            if let Some(s) = self.selected {
                if s > 0 {
                    self.apply_move(s, s - 1);
                }
            }
        } else if (mods.alt || mods.ctrl)
            && ui.input_mut(|i| i.consume_key(mods, egui::Key::ArrowDown))
        {
            if let Some(s) = self.selected {
                if s + 1 < n {
                    self.apply_move(s, s + 2);
                }
            }
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            if let Some(s) = self.selected {
                self.apply_remove(s);
            }
        }
    }

    fn image_list(&mut self, ui: &mut egui::Ui, cx: &Ctx, max_height: Option<f32>, id_salt: &str) {
        let n = self.st.images.len();
        let decode_side = self.list_ui.thumb_size.decode_side();
        let thumb_w = self.list_ui.thumb_size.pixels();
        let thumb_h = thumb_w * 0.75;
        let per_s = self.per_image_secs(cx);
        let hovering = files_hovering(ui.ctx());
        let list_id = ui.id().with(id_salt);
        let frame = egui::Frame::group(ui.style()).inner_margin(6.0);
        let out = frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if n == 0 {
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Drop images or a folder here, or use Add images…").size(15.0));
                    hint(ui, "Drag the grip or a row to reorder, or use ⬆ ⬇. Sort by name, modified date, or Reverse.");
                });
                ui.add_space(18.0);
                return;
            }
            let mut action: Option<(usize, usize)> = None;
            let mut remove: Option<usize> = None;
            let mut select: Option<usize> = None;
            let mut scroll = egui::ScrollArea::vertical().id_salt(id_salt);
            if let Some(h) = max_height {
                scroll = scroll.max_height(h);
            }
            let show_names = self.list_ui.show_filenames;
            let show_thumbs = self.list_ui.show_thumbnails;
            scroll.show(ui, |ui| {
                for i in 0..n {
                    let path = PathBuf::from(&self.st.images[i]);
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let dim_txt = match self.dims.get(&path) {
                        Some(DimProbe::Done(Ok((w, h)))) => format!("{w}×{h}"),
                        Some(DimProbe::Busy(_)) | None => "…".into(),
                        Some(DimProbe::Done(Err(_))) => "!".into(),
                    };
                    if show_thumbs {
                        let need = match self.thumbs.get(&path) {
                            Some(t) if t.mtime == modified(&path) && t.decode_side == decode_side => false,
                            _ => true,
                        };
                        if need {
                            if let Some(t) = load_thumb(cx.egui, &path, decode_side) {
                                self.thumbs.insert(path.clone(), t);
                            }
                        }
                    }
                    let selected = self.selected == Some(i);
                    let fill = if selected { ui.visuals().selection.bg_fill } else { ui.visuals().faint_bg_color };
                    let row_frame = egui::Frame::NONE.inner_margin(6.0).corner_radius(4.0).fill(fill);
                    let row = row_frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let h = drag_handle(ui);
                            h.dnd_set_drag_payload(i);
                            ui.label(RichText::new(format!("{:>2}.", i + 1)).monospace().strong());
                            if icon_button(ui, i > 0, "⬆", &format!("Move image {} up", i + 1)).clicked() {
                                action = Some((i, i - 1));
                            }
                            if icon_button(ui, i + 1 < n, "⬇", &format!("Move image {} down", i + 1)).clicked() {
                                action = Some((i, i + 2));
                            }
                            if icon_button(ui, true, "✖", &format!("Remove image {} from the list", i + 1)).clicked() {
                                remove = Some(i);
                            }
                            if show_thumbs {
                                if let Some(t) = self.thumbs.get(&path) {
                                    ui.add(egui::Image::new((t.tex.id(), egui::vec2(thumb_w, thumb_h))));
                                } else {
                                    ui.allocate_ui(egui::vec2(thumb_w, thumb_h), |ui| {
                                        ui.centered_and_justified(|ui| ui.label("?"));
                                    });
                                }
                            }
                            ui.vertical(|ui| {
                                if show_names {
                                    ui.label(RichText::new(&name).strong()).on_hover_text(path.display().to_string());
                                }
                                let mut details = dim_txt.clone();
                                if let Some(d) = per_s {
                                    if !details.is_empty() {
                                        details.push_str(" · ");
                                    }
                                    details.push_str(&format!("{} s", fmt_num(d)));
                                }
                                if !details.is_empty() {
                                    ui.label(RichText::new(details).weak());
                                }
                                if !show_names {
                                    // still offer the path on hover of the details / index row
                                }
                            });
                        });
                    });
                    let r = row.response.interact(egui::Sense::click());
                    let acc = if show_names {
                        format!("Image {}: {name}, {dim_txt}", i + 1)
                    } else {
                        format!("Image {}: {name}, {dim_txt}", i + 1)
                    };
                    let r = named(r, egui::WidgetType::Button, &acc);
                    if r.clicked() {
                        select = Some(i);
                    }
                    if let (Some(pos), Some(from)) = (ui.ctx().pointer_interact_pos(), r.dnd_hover_payload::<usize>()) {
                        let before = if pos.y < r.rect.center().y { i } else { i + 1 };
                        let y = if before == i { r.rect.top() } else { r.rect.bottom() };
                        ui.painter().hline(r.rect.x_range(), y, egui::Stroke::new(3.0_f32, crate::ui::theme::c().accent));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        if r.dnd_release_payload::<usize>().is_some() {
                            action = Some((*from, before));
                        }
                    }
                }
            });
            if let Some(i) = select {
                self.selected = Some(i);
                ui.memory_mut(|m| m.request_focus(list_id));
            }
            if let Some((from, before)) = action {
                self.apply_move(from, before);
                ui.memory_mut(|m| m.request_focus(list_id));
            }
            if let Some(i) = remove {
                self.apply_remove(i);
                ui.memory_mut(|m| m.request_focus(list_id));
            }
        });
        // Clicking the list frame focuses it for keyboard navigation.
        let frame_r = out.response.interact(egui::Sense::click());
        if frame_r.clicked() {
            ui.memory_mut(|m| m.request_focus(list_id));
        }
        self.handle_list_keys(ui, list_id);
        if hovering {
            paint_drop_target(ui, out.response.rect);
        }
    }

    fn compact_summary(&self, ui: &mut egui::Ui) {
        let n = self.st.images.len();
        if n == 0 {
            hint(ui, "The image list is in a separate window. Add images here or there.");
            return;
        }
        let first = Path::new(&self.st.images[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let last = Path::new(&self.st.images[n - 1]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let line = if n == 1 {
            format!("1 image in the list window: {first}")
        } else {
            format!("{n} images in the list window ({first} … {last})")
        };
        hint(ui, line);
    }

    fn show_popout(&mut self, cx: &Ctx) {
        if !self.list_ui.list_popped_out {
            return;
        }
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Slideshow images")
            .with_min_inner_size([360.0, 240.0])
            .with_resizable(true)
            .with_close_button(true)
            .with_drag_and_drop(true);
        if self.popout_apply_size {
            builder = builder.with_inner_size([self.list_ui.popout_w, self.list_ui.popout_h]);
            self.popout_apply_size = false;
        }
        let id = egui::ViewportId::from_hash_of("slideshow_images");
        let mut close = false;
        cx.egui.show_viewport_immediate(id, builder, |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                let mut open = true;
                let resp = egui::Window::new("Slideshow images")
                    .id(egui::Id::new("slideshow_images_win"))
                    .open(&mut open)
                    .resizable(true)
                    .collapsible(false)
                    .default_size([self.list_ui.popout_w, self.list_ui.popout_h])
                    .show(ctx, |ui| {
                        self.list_toolbar(ui, cx, true);
                        hint(ui, "Drag the grip or a row to reorder. Arrow keys move the selection; Alt+Up / Alt+Down (or Ctrl) reorder; Delete removes the selected row.");
                        let h = ui.available_height().max(160.0);
                        self.image_list(ui, cx, Some(h), "ss_list_pop");
                    });
                if let Some(r) = resp {
                    let s = r.response.rect.size();
                    if s.x > 80.0 && s.y > 80.0 {
                        self.list_ui.popout_w = s.x;
                        self.list_ui.popout_h = s.y;
                    }
                }
                if !open {
                    close = true;
                }
            } else {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.list_toolbar(ui, cx, true);
                    hint(ui, "Drag the grip or a row to reorder. Arrow keys move the selection; Alt+Up / Alt+Down (or Ctrl) reorder; Delete removes the selected row.");
                    ui.add_space(4.0);
                    let h = ui.available_height().max(160.0);
                    self.image_list(ui, cx, Some(h), "ss_list_pop");
                });
                if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
                    let s = rect.size();
                    if s.x > 80.0 && s.y > 80.0 {
                        self.list_ui.popout_w = s.x;
                        self.list_ui.popout_h = s.y;
                    }
                }
                if ctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
            }
        });
        if close {
            self.list_ui.list_popped_out = false;
            self.persist_list_ui(true);
        }
    }
}

impl Drop for SlideshowMode {
    fn drop(&mut self) {
        self.persist_list_ui(true);
    }
}

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

        section(ui, "Images");
        self.list_toolbar(ui, cx, false);
        hint(ui, "Vertical list: drag the grip or a row to reorder, or use ⬆ ⬇. Drop a folder or images onto the window.");
        if self.list_ui.list_popped_out {
            self.compact_summary(ui);
            self.show_popout(cx);
        } else {
            self.image_list(ui, cx, Some(280.0), "ss_list");
        }
        self.persist_list_ui(false);

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
            row_label_tip(ui, "Mode", "How long the slideshow lasts. Per image (default): each image is shown un-faded for a set time. Fixed total: you set the whole length, including transitions. Match audio: the length follows the audio file.");
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

            if let Ok(images) = self.image_inputs() {
                let pr = self.params_preview(images, cx);
                match slideshow::timing(&pr) {
                    Ok(t) => {
                        row_label(ui, "Result");
                        let fade_note = if t.fade_s > 0.0 {
                            format!(" · total = N×{}s + (N−1)×{}s transition", fmt_num(t.per_image_s), fmt_num(t.fade_s))
                        } else {
                            String::new()
                        };
                        hint(ui, format!("Each image fully visible for {} s · total {}{fade_note}", fmt_num(t.per_image_s), fmt_hms(t.total_s)));
                        ui.end_row();
                    }
                    Err(e) => {
                        row_label(ui, "Result");
                        warn(ui, e);
                        ui.end_row();
                    }
                }
            }

            section(ui, "Transition");
            ui.end_row();
            let xfade_ok = cx.has_xfade.unwrap_or(false);
            if cx.has_xfade == Some(false) {
                warn(ui, "This FFmpeg build has no xfade filter, so only hard cuts are available (each image keeps its display time; transition time is not added). Help › Check for FFmpeg updates… can download a full GPL build that includes crossfades.");
                ui.end_row();
                ui.label("");
            }
            row_label_tip(ui, "Between images", "Crossfades need the FFmpeg xfade filter. If it is missing, transitions fall back to hard cuts and transition time is not added (you will see a warning).");
            ui.vertical(|ui| {
                for tr in Transition::ALL {
                    let enabled = !tr.needs_xfade() || xfade_ok || cx.has_xfade.is_none();
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.radio_value(&mut self.st.transition, tr, tr.label());
                    });
                }
                if self.st.transition.needs_xfade() && !xfade_ok && cx.has_xfade.is_some() {
                    hint(ui, "Selected transition needs xfade — this run will use hard cuts (each image keeps its display time; transition time is not added).");
                }
            });
            ui.end_row();
            row_label_tip(ui, "Crossfade length", "Extra time added between two images; each image stays fully visible for its display time, then the transition runs. With N images: total = N×per-image + (N−1)×transition. Example: 3 images of 5 s with a 2 s fade last 19 s.");
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
        self.selected = None;
        // list display prefs are stored separately and survive File › New
        self.popout_apply_size = self.list_ui.list_popped_out;
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
