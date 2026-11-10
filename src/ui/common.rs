//! Shared UI helpers: background tasks, ffprobe slots, path pickers and the
//! reusable video/audio/compressor editors.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, RichText, Ui};
use ffenc::cmd::common::*;
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::util::fmt_hms;

/// Result of a closure run on a background thread.
pub struct Bg<T> {
    rx: Receiver<T>,
}

impl<T: Send + 'static> Bg<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            c.request_repaint();
        });
        Bg { rx }
    }
    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

/// Probes a file in the background whenever its path changes.
#[derive(Default)]
pub struct ProbeSlot {
    probed_path: Option<PathBuf>,
    task: Option<Bg<Result<MediaInfo, String>>>,
    pub result: Option<Result<MediaInfo, String>>,
}

impl ProbeSlot {
    /// Call every frame. Starts a probe when `path` changed and exists. Returns true when a new result arrived.
    pub fn update(&mut self, ctx: &egui::Context, ffprobe: Option<&Path>, path: &str) -> bool {
        let p = PathBuf::from(path.trim());
        let mut arrived = false;
        if let Some(t) = &self.task {
            if let Some(r) = t.poll() {
                self.result = Some(r);
                self.task = None;
                arrived = true;
            }
        }
        if self.probed_path.as_ref() != Some(&p) {
            self.probed_path = Some(p.clone());
            self.result = None;
            self.task = None;
            if !path.trim().is_empty() && p.is_file() {
                if let Some(fp) = ffprobe {
                    let fp = fp.to_path_buf();
                    self.task = Some(Bg::spawn(ctx, move || probe::probe(&fp, &p)));
                } else {
                    self.result = Some(Err("FFprobe not found – set the FFmpeg folder in Settings.".into()));
                }
            }
        }
        arrived
    }
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }
    pub fn info(&self) -> Option<&MediaInfo> {
        self.result.as_ref().and_then(|r| r.as_ref().ok())
    }
    /// Info or an error message suitable for the "build" step.
    pub fn require(&self, what: &str) -> Result<&MediaInfo, String> {
        match &self.result {
            Some(Ok(i)) => Ok(i),
            Some(Err(e)) => Err(format!("{what}: {e}")),
            None if self.busy() => Err(format!("{what}: still probing, try again in a moment")),
            None => Err(format!("{what}: choose an existing file")),
        }
    }
    /// Small status line under a file field.
    pub fn status_ui(&self, ui: &mut Ui, describe: impl FnOnce(&MediaInfo) -> String) {
        if self.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("probing…");
            });
        } else if let Some(r) = &self.result {
            match r {
                Ok(i) => {
                    ui.label(RichText::new(describe(i)).small().color(crate::ui::theme::c().ok));
                }
                Err(e) => {
                    ui.label(RichText::new(e).small().color(crate::ui::theme::c().err));
                }
            }
        }
    }
}

/// One-line summary of a video file.
pub fn describe_video(i: &MediaInfo) -> String {
    match i.video_summary() {
        Ok(v) => format!(
            "{} ({:.2}s) · {}x{} · {} audio track(s) · {}",
            fmt_hms(v.duration_s),
            v.duration_s,
            v.width,
            v.height,
            v.audio_count,
            if v.interlaced { format!("interlaced ({})", v.field_order.clone().unwrap_or_default()) } else { "progressive".into() }
        ),
        Err(e) => e,
    }
}

pub fn describe_audio(i: &MediaInfo) -> String {
    let a = i.of_type("audio").next();
    match a {
        Some(a) => format!(
            "{} · {} · {} Hz · {} ch",
            i.any_duration().map(fmt_hms).unwrap_or_else(|| "?".into()),
            a.codec_name,
            a.sample_rate.map(|s| s.to_string()).unwrap_or_else(|| "?".into()),
            a.channels.map(|s| s.to_string()).unwrap_or_else(|| "?".into())
        ),
        None => "No audio stream found".into(),
    }
}

#[derive(Clone, Copy)]
pub enum Pick<'a> {
    File(&'a str, &'a [&'a str]),
    Folder,
}

fn start_dir(current: &str, fallback: &str) -> Option<PathBuf> {
    let c = PathBuf::from(current.trim());
    let d = if c.is_dir() { Some(c) } else { c.parent().map(|p| p.to_path_buf()).filter(|p| p.is_dir()) };
    d.or_else(|| Some(PathBuf::from(fallback)).filter(|p| p.is_dir()))
}

/// Keep a window inside the screen at any text size (large UI scales leave few
/// points of screen): capped to the screen with a margin. `scroll` adds a
/// vertical scrollbar for dialogs that have no scroll area of their own.
pub fn fit<'a>(w: egui::Window<'a>, ctx: &egui::Context, scroll: bool) -> Fitted<'a> {
    let s = ctx.screen_rect().size();
    let w = w.max_width((s.x - 24.0).max(200.0)).max_height((s.y - 24.0).max(150.0)).constrain(true);
    Fitted { w: if scroll { w.vscroll(true) } else { w }, remember: None }
}

/// A window from [`fit`], optionally remembering its size (window layout).
pub struct Fitted<'a> {
    w: egui::Window<'a>,
    /// Remember this resizable dialog's size (and position when movable) under a layout key.
    remember: Option<(String, bool)>,
}

impl<'a> Fitted<'a> {
    /// Remember the dialog's size across launches (window layout); `movable` also keeps its position
    /// (not for dialogs that are always centered).
    pub fn remember(mut self, key: &str, movable: bool) -> Self {
        self.w = crate::ui::layout::dialog_window(self.w, key, movable);
        self.remember = Some((key.to_string(), movable));
        self
    }

    pub fn show<R>(self, ctx: &egui::Context, add: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<Option<R>>> {
        let Some((key, movable)) = self.remember.clone() else { return self.w.show(ctx, add) };
        let mut content = None;
        let r = self.w.show(ctx, |ui| {
            content = Some(ui.max_rect().size());
            add(ui)
        });
        if let (Some(r), Some(size)) = (&r, content) {
            crate::ui::layout::record_dialog(&key, size, r.response.rect.min, movable);
        }
        r
    }
}

/// Give a widget an accessible name (read by Narrator / NVDA through AccessKit) and
/// the same text as tooltip. Use for anything whose visible text is only a symbol.
pub fn named(resp: egui::Response, kind: egui::WidgetType, name: &str) -> egui::Response {
    let (n, en) = (name.to_string(), resp.enabled());
    resp.widget_info(|| egui::WidgetInfo::labeled(kind, en, &n));
    resp.on_hover_text(name)
}

/// Small icon-only button (⬆ ⬇ ✖ …) with an accessible name and tooltip.
pub fn icon_button(ui: &mut Ui, enabled: bool, icon: &str, name: &str) -> egui::Response {
    let r = ui.add_enabled(enabled, egui::Button::new(icon).small());
    named(r, egui::WidgetType::Button, name)
}

/// True while the OS is dragging files over the window.
pub fn files_hovering(ctx: &egui::Context) -> bool {
    ctx.input(|i| !i.raw.hovered_files.is_empty())
}

/// Highlight a drop target (accent border + faint fill) while files hover the window.
pub fn paint_drop_target(ui: &Ui, rect: egui::Rect) {
    // solid (no pulsing animation), 3 px, accent colour (≥ 3:1 in every theme)
    let c = crate::ui::theme::c().accent;
    ui.painter().rect(rect.expand(2.0), 4.0, c.gamma_multiply(0.10), egui::Stroke::new(3.0_f32, c), egui::StrokeKind::Outside);
}

/// Label + text field + Browse button. The field is a drop target: it is
/// highlighted while files are dragged over the window (the drop itself is
/// routed by the window to the active preset). Returns true when the value changed.
pub fn path_row(ui: &mut Ui, label: &str, value: &mut String, pick: Pick, fallback_dir: &str) -> bool {
    path_row_impl(ui, label, value, pick, fallback_dir, true)
}

/// Like [`path_row`] but not a drop target (output folders, settings).
pub fn path_row_nodrop(ui: &mut Ui, label: &str, value: &mut String, pick: Pick, fallback_dir: &str) -> bool {
    path_row_impl(ui, label, value, pick, fallback_dir, false)
}

fn path_row_impl(ui: &mut Ui, label: &str, value: &mut String, pick: Pick, fallback_dir: &str, droppable: bool) -> bool {
    let mut changed = false;
    row_label(ui, label);
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 80.0).max(120.0);
        let mut te = egui::TextEdit::singleline(value).desired_width(w);
        if droppable {
            te = te.hint_text(match pick {
                Pick::File(..) => "Drop a file here, or Browse…",
                Pick::Folder => "Drop a folder here, or Browse…",
            });
        }
        let r = ui.add(te);
        changed |= r.changed();
        if droppable && files_hovering(ui.ctx()) {
            paint_drop_target(ui, r.rect);
        }
        if ui.button("Browse…").clicked() {
            let mut d = rfd::FileDialog::new();
            if let Some(s) = start_dir(value, fallback_dir) {
                d = d.set_directory(s);
            }
            let picked = match pick {
                Pick::File(name, exts) => d.add_filter(name, exts).add_filter("All files", &["*"]).pick_file(),
                Pick::Folder => d.pick_folder(),
            };
            if let Some(p) = picked {
                *value = p.to_string_lossy().into_owned();
                changed = true;
            }
        }
    });
    ui.end_row();
    changed
}

/// Output folder + file name rows.
pub fn output_rows(ui: &mut Ui, dir: &mut String, name: &mut String, fallback_dir: &str) {
    path_row_nodrop(ui, "Output folder", dir, Pick::Folder, fallback_dir);
    row_label(ui, "Output file name");
    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
    ui.end_row();
}

/// Output folder + name + suffix rows. Returns true when the suffix changed.
pub fn output_rows_sfx(ui: &mut Ui, dir: &mut String, name: &mut String, suffix: &mut String, fallback_dir: &str) -> bool {
    output_rows(ui, dir, name, fallback_dir);
    row_label(ui, "Name suffix");
    let r = ui.horizontal(|ui| {
        let c = ui.add(egui::TextEdit::singleline(suffix).desired_width(160.0)).changed();
        hint(ui, "added to the input name for the automatic output name");
        c
    });
    ui.end_row();
    r.inner
}

/// Keep an auto-generated name in sync unless the user typed their own.
pub fn auto_name(name: &mut String, last_auto: &mut String, new_auto: String) {
    if name.trim().is_empty() || *name == *last_auto {
        *name = new_auto.clone();
    }
    *last_auto = new_auto;
}

pub fn warn(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(format!("⚠ {}", text.into())).color(crate::ui::theme::c().warn));
}

pub fn hint(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).small().weak());
}

pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
    ui.separator();
}

pub fn num_f64(ui: &mut Ui, v: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64, suffix: &str) -> egui::Response {
    ui.add(egui::DragValue::new(v).range(range).speed(speed).suffix(suffix.to_string()).max_decimals(3))
}

pub fn num_u32(ui: &mut Ui, v: &mut u32, range: std::ops::RangeInclusive<u32>, suffix: &str) -> egui::Response {
    ui.add(egui::DragValue::new(v).range(range).speed(1.0).suffix(suffix.to_string()))
}

pub fn passthrough_ui(ui: &mut Ui, id: &str, p: &mut Passthrough) {
    row_label_tip(ui, "Passthrough", "Copy the video and/or audio stream unchanged instead of re-encoding it (fast, no quality loss); filters and encoder settings for a copied stream are ignored.");
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(p.label()).show_ui(ui, |ui| {
        for v in Passthrough::ALL {
            ui.selectable_value(p, v, v.label());
        }
    });
    ui.end_row();
}

/// CRF / preset / tune rows (inside a Grid).
pub fn video_encode_ui(ui: &mut Ui, id: &str, v: &mut VideoEncode, show_tune: bool) {
    row_label(ui, "CRF");
    ui.horizontal(|ui| {
        num_u32(ui, &mut v.crf, 0..=51, "");
        hint(ui, "lower = better quality / bigger file");
    });
    ui.end_row();
    row_label_tip(ui, "Preset", "x264 speed preset: slower presets compress better (smaller file at the same CRF) but take longer. Does not change the quality target.");
    egui::ComboBox::from_id_salt(format!("{id}_preset")).selected_text(v.preset.clone()).show_ui(ui, |ui| {
        for (i, p) in PRESETS.iter().enumerate() {
            ui.selectable_value(&mut v.preset, p.to_string(), format!("{}: {}", i + 1, p));
        }
    });
    ui.end_row();
    if show_tune {
        row_label_tip(ui, "Tune", "Optional x264 tuning for a kind of content (e.g. film, animation, grain). Leave at none if unsure.");
        let txt = v.tune.clone().unwrap_or_else(|| "none".into());
        egui::ComboBox::from_id_salt(format!("{id}_tune")).selected_text(txt).show_ui(ui, |ui| {
            ui.selectable_value(&mut v.tune, None, "none");
            for (i, t) in TUNES.iter().enumerate() {
                ui.selectable_value(&mut v.tune, Some(t.to_string()), format!("{}: {}", i + 1, t));
            }
        });
        ui.end_row();
        output_fps_ui(ui, &mut v.output_fps);
    }
}

/// "Output frame rate" row (re-encoding forms only).
pub fn output_fps_ui(ui: &mut Ui, fps: &mut String) {
    row_label_tip(ui, "Output frame rate", "Frame rate of the new video. Empty = same as the source (default). A lower rate (e.g. 5 fps for a screen-recorded meeting) drops frames, so motion is less smooth; it often makes the file smaller, but not in proportion. The length, audio and sync stay the same.");
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            named(ui.add(egui::TextEdit::singleline(fps).hint_text("same as source").desired_width(110.0)), egui::WidgetType::TextEdit, "Output frame rate (fps, empty = same as source)");
            ui.label("fps");
            if fps.trim().is_empty() {
                hint(ui, "(empty = same as source)");
            }
        });
        if let Err(e) = ffenc::cmd::common::parse_output_fps(fps) {
            ui.colored_label(crate::ui::theme::c().err, e);
        }
    });
    ui.end_row();
}

pub fn bitrate_ui(ui: &mut Ui, label: &str, k: &mut u32) {
    row_label(ui, label);
    let pal = crate::ui::theme::c();
    ui.vertical(|ui| {
        let readout = format!("Will encode: {k} kb/s");
        let color = if pal.high_contrast { pal.accent } else { ui.visuals().strong_text_color() };
        let r = ui.label(RichText::new(&readout).strong().size(15.0).color(color));
        named(r, egui::WidgetType::Label, &readout);
        ui.add_space(10.0);
        hint(ui, "Available bitrates");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for b in [128, 160, 192, 256, 320] {
                let selected = *k == b;
                let text = if selected {
                    let c = if pal.high_contrast { pal.accent } else { pal.on_pill };
                    RichText::new(format!("{b}")).strong().color(c)
                } else {
                    RichText::new(format!("{b}"))
                };
                let mut btn = egui::Button::new(text);
                if selected {
                    if pal.high_contrast {
                        btn = btn.fill(egui::Color32::BLACK).stroke(egui::Stroke::new(pal.focus_width.max(2.0), pal.accent));
                    } else {
                        btn = btn.fill(pal.accent).stroke(egui::Stroke::new(2.0_f32, pal.accent));
                    }
                }
                let name = if selected { format!("{b} kb/s (selected)") } else { format!("{b} kb/s") };
                if named(ui.add(btn), egui::WidgetType::Button, &name).clicked() {
                    *k = b;
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            hint(ui, "Or type a custom value");
            num_u32(ui, k, 8..=1024, " kbps");
        });
    });
    ui.end_row();
}

/// Bitrate + gain + compressor rows.
pub fn audio_encode_ui(ui: &mut Ui, id: &str, a: &mut AudioEncode) {
    bitrate_ui(ui, "AAC bitrate", &mut a.bitrate_k);
    audio_filters_ui(ui, id, a);
}

/// Gain / compressor rows (any re-encoded audio codec).
pub fn audio_filters_ui(ui: &mut Ui, id: &str, a: &mut AudioEncode) {
        row_label_tip(ui, "Gain", "Volume change applied to the audio, in decibels (+6 dB is roughly twice as loud). 0 = unchanged.");
    ui.horizontal(|ui| {
        num_f64(ui, &mut a.gain_db, -30.0..=30.0, 0.1, " dB");
        hint(ui, "-30..+30");
    });
    ui.end_row();
    row_label_tip(ui, "Compressor", "Evens out loud and quiet parts (FFmpeg acompressor) so a gain boost does not clip.");
    ui.vertical(|ui| {
        egui::ComboBox::from_id_salt(format!("{id}_comp"))
            .selected_text(a.compressor.label())
            .show_ui(ui, |ui| {
                for c in CompressorPreset::ALL {
                    ui.selectable_value(&mut a.compressor, c, format!("{} – {}", c.label(), c.help()));
                }
            });
        if a.compressor != CompressorPreset::None && a.gain_db <= 0.0 {
            hint(ui, "Tip: the compressor is meant to tame peaks before a gain boost (script only offered it with gain > 0).");
        }
        if a.compressor == CompressorPreset::Advanced {
            compressor_params_ui(ui, &format!("{id}_adv"), &mut a.advanced);
        }
        if let Some(c) = CompressorParams::for_preset(a.compressor, &a.advanced) {
            if let Err(e) = c.validate() {
                ui.colored_label(crate::ui::theme::c().err, e);
            }
        }
    });
    ui.end_row();
}

pub fn compressor_params_ui(ui: &mut Ui, id: &str, c: &mut CompressorParams) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        row_label_tip(ui, "Threshold", "Level above which the compressor starts reducing the volume.");
        num_f64(ui, &mut c.threshold_db, -60.0..=0.0, 0.1, " dB");
        ui.end_row();
        row_label_tip(ui, "Ratio", "How strongly levels above the threshold are reduced (4:1 = 4 dB over becomes 1 dB over).");
        num_f64(ui, &mut c.ratio, 1.0..=20.0, 0.05, ":1");
        ui.end_row();
        row_label_tip(ui, "Knee", "Softens the transition around the threshold (1 = hard, higher = gentler).");
        num_f64(ui, &mut c.knee, 1.0..=8.0, 0.05, "");
        ui.end_row();
        row_label_tip(ui, "Makeup", "Gain added after compression to make up for the reduced level.");
        num_f64(ui, &mut c.makeup_db, -12.0..=12.0, 0.1, " dB");
        ui.end_row();
        row_label_tip(ui, "Attack", "How quickly the compressor reacts when the level rises above the threshold.");
        num_f64(ui, &mut c.attack_ms, 0.01..=2000.0, 0.5, " ms");
        ui.end_row();
        row_label_tip(ui, "Release", "How quickly the compressor lets go after the level drops below the threshold.");
        num_f64(ui, &mut c.release_ms, 1.0..=9000.0, 1.0, " ms");
        ui.end_row();
    });
}

pub fn range_ui(ui: &mut Ui, v: &mut bool) {
    row_label(ui, "Colour range");
    ui.vertical(|ui| {
        ui.checkbox(v, "Convert full-range to limited-range BT.709 and tag it");
        hint(ui, "Use for full-range PC/capture sources. Skip if the source is already limited/video range (avoids grey blacks).");
    });
    ui.end_row();
}

pub fn faststart_ui(ui: &mut Ui, v: &mut bool, enabled: bool) {
    row_label(ui, "Faststart");
    ui.add_enabled(enabled, egui::Checkbox::new(v, "Move index to the front for faster streaming (MP4 only)"));
    ui.end_row();
}

/// Dropped paths helpers.
pub fn first_with_ext<'a>(paths: &'a [PathBuf], exts: &[&str]) -> Option<&'a PathBuf> {
    paths.iter().find(|p| ffenc::util::has_ext(p, exts))
}

pub fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Validate an input file exists and has one of the extensions.
pub fn check_input(path: &str, exts: &[&str], what: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err(format!("Choose the {what}."));
    }
    if !p.is_file() {
        return Err(format!("File not found: {}", p.display()));
    }
    if !ffenc::util::has_ext(&p, exts) {
        return Err(format!("Unsupported {what}: {} (must be .{})", p.display(), exts.join(", .")));
    }
    Ok(p)
}

/// Resolve output folder + name; creates nothing (the runner creates the folder).
pub fn check_output(dir: &str, name: &str) -> Result<PathBuf, String> {
    if dir.trim().is_empty() {
        return Err("Choose an output folder.".into());
    }
    let n = name.trim();
    if n.is_empty() {
        return Err("Enter an output file name.".into());
    }
    if n.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(format!("Invalid characters in output file name: {n}"));
    }
    Ok(PathBuf::from(dir.trim()).join(n))
}

/// ffmpeg_error_<name>.log in the output folder.
pub fn log_path_for(output: &Path) -> PathBuf {
    let dir = output.parent().unwrap_or(Path::new("."));
    dir.join(format!("ffmpeg_error_{}.log", ffenc::util::safe_name(&ffenc::util::stem(output))))
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Button with the standard "copy" icon (two overlapping pages, drawn – no icon
/// font needed, so it renders the same on every Windows install) and a text label.
pub fn copy_button(ui: &mut Ui, label: &str, tooltip: &str) -> egui::Response {
    let galley = egui::WidgetText::from(label).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let pad = ui.spacing().button_padding;
    let icon = 14.0;
    let gap = if label.is_empty() { 0.0 } else { 6.0 };
    let size = egui::vec2(pad.x * 2.0 + icon + gap + galley.size().x, galley.size().y.max(icon) + pad.y * 2.0).max(ui.spacing().interact_size);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let vis = ui.style().interact(&resp);
        let p = ui.painter();
        p.rect(rect.expand(vis.expansion), vis.corner_radius, vis.weak_bg_fill, vis.bg_stroke, egui::StrokeKind::Inside);
        let stroke = egui::Stroke::new(1.3_f32, vis.fg_stroke.color);
        let top_left = egui::pos2(rect.left() + pad.x, rect.center().y - icon / 2.0);
        let page = egui::vec2(9.0, 11.0);
        let back = egui::Rect::from_min_size(top_left, page);
        let front = back.translate(egui::vec2(4.5, 3.0));
        p.rect_stroke(back, 1.5, stroke, egui::StrokeKind::Inside);
        p.rect_filled(front, 1.5, vis.weak_bg_fill);
        p.rect_stroke(front, 1.5, stroke, egui::StrokeKind::Inside);
        let text_pos = egui::pos2(rect.left() + pad.x + icon + gap, rect.center().y - galley.size().y / 2.0);
        p.galley(text_pos, galley, vis.text_color());
    }
    named(resp, egui::WidgetType::Button, tooltip)
}

/// Label in the first column of a form grid, aligned to the **top** of its row (egui's Grid
/// centres cells vertically, so a label next to a tall control would float in the middle).
/// The label takes its normal space in the grid (so row heights are unchanged) but is drawn at
/// the top of the row, where the cursor is when the row starts. Hover / accessibility use the
/// drawn position.
pub fn row_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let text: egui::WidgetText = text.into();
    let row_top = ui.cursor().top();
    let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    let drawn = egui::Rect::from_min_size(egui::pos2(rect.left(), row_top.min(rect.top())), galley.size());
    let resp = ui.interact(drawn, ui.id().with(("row_label", rect.left_top().x as i32, row_top as i32)), egui::Sense::hover());
    if ui.is_rect_visible(drawn) {
        let color = ui.style().visuals.text_color();
        ui.painter().galley(drawn.min, galley.clone(), color);
    }
    let t = galley.text().to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &t));
    resp
}

/// [`row_label`] with a hover explanation (shown on the label) for options without their own help text.
pub fn row_label_tip(ui: &mut Ui, text: &str, tip: &str) -> egui::Response {
    row_label(ui, text).on_hover_text(tip)
}

#[cfg(test)]
mod row_label_tests {
    use super::*;

    /// Two rows (tall control in both, the 2nd is the grid's last row): labels sit at their row's
    /// top, the grid has the same height as with plain labels, and nothing grows across frames.
    #[test]
    fn row_labels_are_top_aligned_and_stable() {
        fn frame(ctx: &egui::Context, top: bool) -> (Vec<f32>, Vec<f32>, f32) {
            let (mut labels, mut rows, mut h) = (vec![], vec![], 0.0);
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = egui::Grid::new("g").num_columns(2).show(ui, |ui| {
                        for name in ["First", "Last"] {
                            rows.push(ui.cursor().top());
                            let l = if top { row_label(ui, name) } else { ui.label(name) };
                            labels.push(l.rect.top());
                            ui.vertical(|ui| {
                                for _ in 0..4 {
                                    ui.label("line");
                                }
                            });
                            ui.end_row();
                        }
                    });
                    h = r.response.rect.height();
                });
            });
            (labels, rows, h)
        }
        let plain = egui::Context::default();
        let ctx = egui::Context::default();
        let mut last = None;
        for _ in 0..6 {
            frame(&plain, false);
            last = Some((frame(&ctx, true), frame(&plain, false)));
        }
        let ((labels, rows, h), (plain_labels, _, plain_h)) = last.unwrap();
        assert_eq!(h, plain_h, "row sizing unchanged");
        for i in 0..2 {
            assert!((labels[i] - rows[i]).abs() < 0.5, "row {i} label at the row top: {} vs {}", labels[i], rows[i]);
            assert!(plain_labels[i] > rows[i] + 10.0, "plain labels are centred (the problem being fixed)");
        }
        // stable across more frames
        let again = frame(&ctx, true);
        assert_eq!(again.2, h);
    }
}
