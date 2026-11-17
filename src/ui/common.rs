//! Shared UI helpers: background tasks, ffprobe slots, path pickers and the
//! reusable video/audio/compressor editors.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, RichText, Ui};
use ffenc::cmd::codecs::{AAC_BITRATES, VideoCodec};
use ffenc::cmd::crf;
use ffenc::cmd::common::*;
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::util::fmt_hms;

use super::option_tips;

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

/// Heading that is safe to use as the first cell of a 2-column Grid row (no `add_space` / separator).
pub fn grid_section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).strong());
    ui.label("");
    ui.end_row();
}

/// Hover text wrapped to a modest width so large text size doesn't bury the panel.
pub fn hover_tip(r: egui::Response, tip: impl Into<String>) -> egui::Response {
    let tip = tip.into();
    r.on_hover_ui(|ui| {
        let w = (ui.text_style_height(&egui::TextStyle::Body) * 22.0).clamp(280.0, 480.0);
        ui.set_max_width(w);
        ui.label(tip);
    })
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


/// Clickable (?) next to CRF; opens a short pop-up. Never changes the value.
pub fn crf_help_button(ui: &mut Ui, id: &str, codec: VideoCodec, height: Option<u32>) {
    let open_id = ui.id().with(("crf_help", id, codec.short()));
    let r = ui.add(egui::Button::new("(?)").small()).on_hover_text("What CRF means, and a suggested range for this video's size");
    if r.clicked() {
        ui.data_mut(|d| d.insert_temp(open_id, true));
    }
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    if open {
        egui::Window::new("About CRF")
            .id(open_id)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::LEFT_TOP)
            .default_pos(r.rect.right_top() + egui::vec2(8.0, 0.0))
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.set_max_width(420.0);
                ui.label(crf::popup_text(codec, height));
            });
        ui.data_mut(|d| d.insert_temp(open_id, open));
    }
}

/// CRF / preset / tune rows (inside a Grid).
pub fn video_encode_ui(ui: &mut Ui, id: &str, v: &mut VideoEncode, show_tune: bool, show_output_fps: bool, src_height: Option<u32>) {
    row_label_tip(ui, "CRF", option_tips::CRF);
    ui.horizontal(|ui| {
        num_u32(ui, &mut v.crf, 0..=51, "");
        crf_help_button(ui, id, VideoCodec::H264, src_height);
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
            let extra = v.tune.as_ref().filter(|t| !TUNES.contains(&t.as_str()) && !t.is_empty()).cloned();
            let tuns: Vec<String> = extra.into_iter().chain(TUNES.iter().map(|s| s.to_string())).collect();
            for (i, tn) in tuns.iter().enumerate() {
                ui.selectable_value(&mut v.tune, Some(tn.clone()), format!("{}: {}", i + 1, tn));
            }
        });
        ui.end_row();
    }
    if show_output_fps {
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

/// Closed drop-down label: a listed rate, or "Custom" when the value is typed / Custom… was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateMenu {
    Listed(u32),
    Custom,
}

pub fn bitrate_menu_selection(value: u32, choices: &[u32], custom_open: bool) -> BitrateMenu {
    if custom_open || !choices.contains(&value) {
        BitrateMenu::Custom
    } else {
        BitrateMenu::Listed(value)
    }
}

/// `picked` is `Some(kbps)` for a typical rate, or `None` for Custom…. The stored value is unchanged when opening Custom.
pub fn apply_bitrate_menu(current: u32, picked: Option<u32>) -> (u32, bool) {
    match picked {
        Some(b) => (b, false),
        None => (current, true),
    }
}

pub fn show_custom_bitrate_field(value: u32, choices: &[u32], custom_open: bool) -> bool {
    matches!(bitrate_menu_selection(value, choices, custom_open), BitrateMenu::Custom)
}

/// AAC (and other standalone AAC rows): typical rates plus Custom….
pub fn bitrate_ui(ui: &mut Ui, label: &str, k: &mut u32) {
    bitrate_ui_ex(ui, label, k, &AAC_BITRATES, 8..=1024);
}

/// Bitrate drop-down of `choices` plus Custom… (typed box next to it). Keeps the "Will encode: N kb/s" readout.
pub fn bitrate_ui_ex(ui: &mut Ui, label: &str, k: &mut u32, choices: &[u32], range: std::ops::RangeInclusive<u32>) {
    row_label(ui, label);
    let pal = crate::ui::theme::c();
    ui.vertical(|ui| {
        let readout = format!("Will encode: {k} kb/s");
        let color = if pal.high_contrast { pal.accent } else { ui.visuals().strong_text_color() };
        let r = ui.label(RichText::new(&readout).strong().size(15.0).color(color));
        named(r, egui::WidgetType::Label, &readout);
        ui.add_space(6.0);
        let custom_id = ui.id().with(("bitrate_custom", label));
        let mut custom_open = ui.data(|d| d.get_temp::<bool>(custom_id)).unwrap_or(false);
        let sel = bitrate_menu_selection(*k, choices, custom_open);
        let selected_text = match sel {
            BitrateMenu::Listed(b) => format!("{b} kb/s"),
            BitrateMenu::Custom => "Custom".to_string(),
        };
        ui.horizontal(|ui| {
            let combo = egui::ComboBox::from_id_salt(format!("bitrate:{label}")).width(140.0).selected_text(RichText::new(&selected_text).strong());
            let r = combo.show_ui(ui, |ui| {
                for &b in choices {
                    let on = sel == BitrateMenu::Listed(b);
                    if ui.selectable_label(on, format!("{b} kb/s")).clicked() {
                        let (v, c) = apply_bitrate_menu(*k, Some(b));
                        *k = v;
                        custom_open = c;
                    }
                }
                if ui.selectable_label(sel == BitrateMenu::Custom, "Custom…").clicked() {
                    let (v, c) = apply_bitrate_menu(*k, None);
                    *k = v;
                    custom_open = c;
                }
            });
            named(r.response, egui::WidgetType::ComboBox, &format!("{label}: {selected_text}. Choose a typical rate, or Custom… to type another value."));
            if show_custom_bitrate_field(*k, choices, custom_open) {
                let nr = num_u32(ui, k, range, " kbps");
                if nr.changed() && choices.contains(k) {
                    custom_open = false;
                }
                named(nr, egui::WidgetType::DragValue, &format!("{label} custom value (kbps)"));
            }
        });
        ui.data_mut(|d| d.insert_temp(custom_id, custom_open));
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
        row_label_tip(ui, "Gain", "Pre-gain in decibels, applied before the compressor (+6 dB is roughly twice as loud). 0 = unchanged.");
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
        row_label_tip(ui, "Ratio", "How strongly levels above the threshold are reduced (4:1 = 4 dB over becomes 1 dB over).");
        num_f64(ui, &mut c.ratio, 1.0..=20.0, 0.05, ":1");
        ui.end_row();
        row_label_tip(ui, "Threshold", "Level above which the compressor starts reducing the volume.");
        num_f64(ui, &mut c.threshold_db, -60.0..=0.0, 0.1, " dB");
        ui.end_row();
        row_label_tip(ui, "Knee", "Softens the transition around the threshold (1 = hard, higher = gentler).");
        num_f64(ui, &mut c.knee, 1.0..=8.0, 0.05, "");
        ui.end_row();
        row_label_tip(ui, "Attack", "How quickly the compressor reacts when the level rises above the threshold.");
        num_f64(ui, &mut c.attack_ms, 0.01..=2000.0, 0.5, " ms");
        ui.end_row();
        row_label_tip(ui, "Release", "How quickly the compressor lets go after the level drops below the threshold.");
        num_f64(ui, &mut c.release_ms, 1.0..=9000.0, 1.0, " ms");
        ui.end_row();
        row_label_tip(ui, "Make-up gain", "Gain added after compression to make up for the reduced level.");
        num_f64(ui, &mut c.makeup_db, -12.0..=12.0, 0.1, " dB");
        ui.end_row();
    });
}

/// Colour-range checkbox. `probed` is `None` when there is no source probe yet,
/// `Some(None)` when the file was probed but the range is missing or unknown
/// (a mild hover-only note, no warning), and `Some(Limited/Full)` when tagged.
pub fn range_ui(ui: &mut Ui, v: &mut bool, probed: Option<Option<ffenc::cmd::common::ColorRange>>) {
    row_label_tip(ui, "Colour range", option_tips::COLOUR_RANGE);
    ui.vertical(|ui| {
        ui.checkbox(v, "Convert full-range to limited-range BT.709 and tag it");
        hint(ui, "Use for full-range PC/capture sources. Skip if the source is already limited/video range (avoids grey blacks).");
        if matches!(probed, Some(None)) {
            hint(ui, "This file's colour range is not tagged. Most captures are already TV range. Check a dark scene.");
        }
    });
    ui.end_row();
    if let Some(c) = ffenc::cmd::conflicts::source_already_limited(*v, probed.flatten()) {
        ui.label("");
        if let ConflictAction::Fix(_) = conflict_warning_ui(ui, &c, false) {
            *v = false;
        }
        ui.end_row();
    }
}

pub fn faststart_ui(ui: &mut Ui, v: &mut bool, enabled: bool) {
    row_label_tip(ui, "Faststart", option_tips::FASTSTART);
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
    hover_tip(row_label(ui, text), tip)
}


/// Result of a one-click warning row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    None,
    Fix(ffenc::cmd::conflicts::ConflictKind),
    Keep(ffenc::cmd::conflicts::ConflictKind),
}

/// Visible strip for a warning: the panel / window clip, not the current Grid cell.
/// Grid cells often report infinite `available_width` on the first pass, which would
/// let unwrapped text widen the column and squeeze later cells.
fn warning_strip(ui: &Ui) -> (f32, f32) {
    let pad = ui.spacing().item_spacing.x.max(4.0);
    let left = ui.clip_rect().left() + pad;
    let right = ui.clip_rect().right() - pad;
    let wrap_w = (right - left).max(ui.spacing().interact_size.x);
    let shift = (ui.cursor().left() - left).max(0.0);
    (wrap_w, shift)
}

fn unwrapped_button(ui: &Ui, label: impl Into<egui::WidgetText>, wrap_w: f32) -> egui::Button<'static> {
    let text = label.into();
    // Keep a natural (one-line) width when it fits; only word-wrap if the button
    // itself is wider than the panel (large text in a very narrow window).
    let natural = text
        .clone()
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::FontSelection::Default)
        .size()
        .x
        + ui.spacing().button_padding.x * 2.0;
    let mode = if natural <= wrap_w { egui::TextWrapMode::Extend } else { egui::TextWrapMode::Wrap };
    egui::Button::new(text).wrap_mode(mode).min_size(egui::vec2(24.0, 32.0))
}

/// Plain-words warning plus optional “fix it” / “keep my setting” buttons.
/// Large, keyboard-focusable buttons; uses the theme warning colour (including High contrast).
///
/// Occupies a single parent cell (safe inside a 2-column Grid). The block is pulled to the
/// panel's left so the text wraps to the full available width; fix / keep buttons sit on
/// the next line at their natural width (no letter-wrapping), left-aligned, wrapping to a
/// further line as whole buttons if they do not fit side by side.
pub fn conflict_warning_ui(ui: &mut Ui, c: &ffenc::cmd::conflicts::Conflict, show_keep: bool) -> ConflictAction {
    let mut action = ConflictAction::None;
    let (wrap_w, shift) = warning_strip(ui);
    let top = ui.cursor().top();
    let cell_left = ui.cursor().left();
    let paint_left = cell_left - shift;
    let paint_rect = egui::Rect::from_min_size(egui::pos2(paint_left, top), egui::vec2(wrap_w, 10_000.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("conflict_warning", c.kind as u32))
            .max_rect(paint_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.add_space(4.0);
    child.add(
        egui::Label::new(RichText::new(format!("⚠ {}", c.message)).color(crate::ui::theme::c().warn)).wrap(),
    );
    if let Some(fix) = c.fix {
        child.horizontal_wrapped(|ui| {
            let r = ui.add(unwrapped_button(ui, RichText::new(fix.button).strong(), wrap_w));
            let r = named(r, egui::WidgetType::Button, &format!("Fix it for me: {}", fix.hint));
            if r.on_hover_text(fix.hint).clicked() {
                action = ConflictAction::Fix(c.kind);
            }
            if show_keep {
                let r = ui.add(unwrapped_button(ui, "Keep my setting", wrap_w));
                let r = named(r, egui::WidgetType::Button, "Keep my setting: leave this as it is");
                if r.clicked() {
                    action = ConflictAction::Keep(c.kind);
                }
            }
        });
    }
    let used_h = child.min_rect().height();
    let pad = ui.spacing().item_spacing.x.max(4.0);
    let cell_w = (ui.clip_rect().right() - pad - cell_left).max(ui.spacing().interact_size.x);
    // Reserve this cell's height so the next Grid row is not drawn under the wrapped text.
    // allocate_exact_size is legal inside a Grid; advance_cursor_after_rect is not and panics.
    ui.allocate_exact_size(egui::vec2(cell_w, used_h), egui::Sense::hover());
    action
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

#[cfg(test)]
mod conflict_warning_layout_tests {
    use super::*;
    use ffenc::cmd::conflicts::{Conflict, ConflictKind, Fix};

    fn sample() -> Conflict {
        Conflict {
            kind: ConflictKind::SourceAlreadyLimited,
            message: "This source is already tagged as TV (limited) range, and a full-to-TV conversion is also on, which can squeeze the picture twice and make blacks look grey. The encode will still run.".into(),
            fix: Some(Fix {
                button: "Turn off the range conversion",
                hint: "Turns off “Convert full-range to limited-range”, so a TV-range source is not squeezed again.",
            }),
        }
    }

    /// Render the colour-range warning in a 2-column form grid, as Re-encode does.
    fn layout(width: f32, text_scale: f32, show_keep: bool) -> (egui::Rect, egui::Rect) {
        let ctx = egui::Context::default();
        if (text_scale - 1.0).abs() > 0.001 {
            ctx.style_mut(|style| {
                for font in style.text_styles.values_mut() {
                    font.size *= text_scale;
                }
            });
        }
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0));
        let mut panel = egui::Rect::NOTHING;
        let mut block = egui::Rect::NOTHING;
        // Two frames: Grid measures on the first pass and positions on the second.
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panel = ui.max_rect();
                    egui::Grid::new("form").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("Colour range");
                        let mut on = true;
                        ui.checkbox(&mut on, "Convert full-range to limited-range BT.709 and tag it");
                        ui.end_row();
                        ui.label("");
                        block = ui.scope(|ui| {
                            let _ = conflict_warning_ui(ui, &sample(), show_keep);
                        }).response.rect;
                        ui.end_row();
                    });
                });
            });
        }
        (panel, block)
    }

    #[test]
    fn conflict_warning_inside_a_real_grid_does_not_panic() {
        // Production forms (range_ui, Frameserver) call this as a Grid cell, not inside ui.scope.
        // advance_cursor_after_rect used to panic: "You cannot advance the cursor when in a grid layout".
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 800.0));
        let mut next_top = 0.0;
        let warn_bottom = 0.0;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::Grid::new("form").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("Colour range");
                        let mut on = true;
                        ui.checkbox(&mut on, "Convert full-range to limited-range");
                        ui.end_row();
                        ui.label("");
                        let _ = conflict_warning_ui(ui, &sample(), true);
                        ui.end_row();
                        next_top = ui.cursor().top();
                        ui.label("Deinterlace");
                        ui.checkbox(&mut on, "Deinterlace");
                        ui.end_row();
                    });
                });
            });
        }
        let _ = (next_top, warn_bottom);
        // If we got here the Grid did not panic. The following row must sit below the warning.
        assert!(next_top > 40.0, "grid reserved height for the warning, next row at {next_top}");
    }

    #[test]
    fn warning_text_wraps_and_fix_button_is_not_squeezed_in_a_narrow_grid() {
        let width = 420.0;
        let (panel, block) = layout(width, 1.0, false);
        let _ = panel;
        assert!(block.right() <= width + 1.0, "warning stays inside the window: block {:?} width {}", block, width);
        assert!(block.width() < width, "text wrapped instead of one long line: {}", block.width());
        // Text + one button row, not a tall stack of one-letter-wrapped button glyphs.
        assert!(block.height() > 40.0, "text and button both shown: {}", block.height());
        assert!(block.height() < 220.0, "button is not letter-wrapped into a narrow column: {}", block.height());
    }

    #[test]
    fn warning_still_fits_with_large_text_and_keep_button() {
        let width = 420.0;
        let (panel, block) = layout(width, 1.75, true);
        let _ = panel;
        assert!(block.right() <= width + 1.0, "large-text warning stays inside the window: block {:?} width {}", block, width);
        // A letter-wrapped “Turn off the range conversion” button would be many lines tall.
        assert!(block.height() < 420.0, "buttons keep a natural width at large text: {}", block.height());
    }
}

#[cfg(test)]
mod bitrate_menu_tests {
    use super::*;
    use ffenc::cmd::codecs::{AAC_BITRATES, AC3_BITRATES};

    #[test]
    fn typical_aac_and_ac3_lists() {
        assert_eq!(AAC_BITRATES, [96, 128, 160, 192, 256, 320]);
        assert_eq!(AC3_BITRATES, [192, 256, 384, 448, 640]);
        assert!(AAC_BITRATES.contains(&192), "default AAC rate is in the list");
        assert!(AC3_BITRATES.contains(&448), "default AC3 rate is in the list");
    }

    #[test]
    fn listed_value_shows_as_itself_until_custom_is_chosen() {
        assert_eq!(bitrate_menu_selection(192, &AAC_BITRATES, false), BitrateMenu::Listed(192));
        assert_eq!(bitrate_menu_selection(96, &AAC_BITRATES, false), BitrateMenu::Listed(96));
        assert_eq!(bitrate_menu_selection(320, &AAC_BITRATES, false), BitrateMenu::Listed(320));
        assert!(!show_custom_bitrate_field(192, &AAC_BITRATES, false));
    }

    #[test]
    fn value_not_in_the_list_is_custom_and_shows_the_box() {
        assert_eq!(bitrate_menu_selection(200, &AAC_BITRATES, false), BitrateMenu::Custom);
        assert_eq!(bitrate_menu_selection(64, &AAC_BITRATES, false), BitrateMenu::Custom);
        assert!(show_custom_bitrate_field(200, &AAC_BITRATES, false));
        assert_eq!(bitrate_menu_selection(320, &AC3_BITRATES, false), BitrateMenu::Custom);
        assert_eq!(bitrate_menu_selection(448, &AC3_BITRATES, false), BitrateMenu::Listed(448));
    }

    #[test]
    fn choosing_custom_keeps_the_stored_rate_and_opens_the_box() {
        assert_eq!(apply_bitrate_menu(192, None), (192, true));
        assert_eq!(bitrate_menu_selection(192, &AAC_BITRATES, true), BitrateMenu::Custom);
        assert!(show_custom_bitrate_field(192, &AAC_BITRATES, true));
    }

    #[test]
    fn choosing_a_listed_rate_stores_it_and_closes_custom() {
        assert_eq!(apply_bitrate_menu(200, Some(256)), (256, false));
        assert_eq!(apply_bitrate_menu(192, Some(96)), (96, false));
        assert!(!show_custom_bitrate_field(256, &AAC_BITRATES, false));
    }
}
