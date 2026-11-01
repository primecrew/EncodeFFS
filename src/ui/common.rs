//! Shared UI helpers: background tasks, ffprobe slots, path pickers and the
//! reusable video/audio/compressor editors.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use eframe::egui::{self, Color32, RichText, Ui};
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
                    ui.label(RichText::new(describe(i)).small().color(Color32::from_rgb(120, 180, 120)));
                }
                Err(e) => {
                    ui.label(RichText::new(e).small().color(Color32::LIGHT_RED));
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

/// Label + text field + Browse button. Returns true when the value changed.
pub fn path_row(ui: &mut Ui, label: &str, value: &mut String, pick: Pick, fallback_dir: &str) -> bool {
    let mut changed = false;
    ui.label(label);
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 80.0).max(120.0);
        changed |= ui.add(egui::TextEdit::singleline(value).desired_width(w)).changed();
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
    path_row(ui, "Output folder", dir, Pick::Folder, fallback_dir);
    ui.label("Output file name");
    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
    ui.end_row();
}

/// Keep an auto-generated name in sync unless the user typed their own.
pub fn auto_name(name: &mut String, last_auto: &mut String, new_auto: String) {
    if name.trim().is_empty() || *name == *last_auto {
        *name = new_auto.clone();
    }
    *last_auto = new_auto;
}

pub fn warn(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(format!("⚠ {}", text.into())).color(Color32::from_rgb(230, 180, 60)));
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
    ui.label("Passthrough");
    egui::ComboBox::from_id_salt(id).width(300.0).selected_text(p.label()).show_ui(ui, |ui| {
        for v in Passthrough::ALL {
            ui.selectable_value(p, v, v.label());
        }
    });
    ui.end_row();
}

/// CRF / preset / tune rows (inside a Grid).
pub fn video_encode_ui(ui: &mut Ui, id: &str, v: &mut VideoEncode, show_tune: bool) {
    ui.label("CRF");
    ui.horizontal(|ui| {
        num_u32(ui, &mut v.crf, 0..=51, "");
        hint(ui, "lower = better quality / bigger file");
    });
    ui.end_row();
    ui.label("Preset");
    egui::ComboBox::from_id_salt(format!("{id}_preset")).selected_text(v.preset.clone()).show_ui(ui, |ui| {
        for (i, p) in PRESETS.iter().enumerate() {
            ui.selectable_value(&mut v.preset, p.to_string(), format!("{}: {}", i + 1, p));
        }
    });
    ui.end_row();
    if show_tune {
        ui.label("Tune");
        let txt = v.tune.clone().unwrap_or_else(|| "none".into());
        egui::ComboBox::from_id_salt(format!("{id}_tune")).selected_text(txt).show_ui(ui, |ui| {
            ui.selectable_value(&mut v.tune, None, "none");
            for (i, t) in TUNES.iter().enumerate() {
                ui.selectable_value(&mut v.tune, Some(t.to_string()), format!("{}: {}", i + 1, t));
            }
        });
        ui.end_row();
    }
}

pub fn bitrate_ui(ui: &mut Ui, label: &str, k: &mut u32) {
    ui.label(label);
    ui.horizontal(|ui| {
        num_u32(ui, k, 8..=1024, " kbps");
        for b in [128, 160, 192, 256, 320] {
            if ui.small_button(b.to_string()).clicked() {
                *k = b;
            }
        }
    });
    ui.end_row();
}

/// Bitrate + gain + compressor rows.
pub fn audio_encode_ui(ui: &mut Ui, id: &str, a: &mut AudioEncode) {
    bitrate_ui(ui, "AAC bitrate", &mut a.bitrate_k);
    ui.label("Gain");
    ui.horizontal(|ui| {
        num_f64(ui, &mut a.gain_db, -30.0..=30.0, 0.1, " dB");
        hint(ui, "-30..+30");
    });
    ui.end_row();
    ui.label("Compressor");
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
                ui.colored_label(Color32::LIGHT_RED, e);
            }
        }
    });
    ui.end_row();
}

pub fn compressor_params_ui(ui: &mut Ui, id: &str, c: &mut CompressorParams) {
    egui::Grid::new(id).num_columns(2).show(ui, |ui| {
        ui.label("Threshold");
        num_f64(ui, &mut c.threshold_db, -60.0..=0.0, 0.1, " dB");
        ui.end_row();
        ui.label("Ratio");
        num_f64(ui, &mut c.ratio, 1.0..=20.0, 0.05, ":1");
        ui.end_row();
        ui.label("Knee");
        num_f64(ui, &mut c.knee, 1.0..=8.0, 0.05, "");
        ui.end_row();
        ui.label("Makeup");
        num_f64(ui, &mut c.makeup_db, -12.0..=12.0, 0.1, " dB");
        ui.end_row();
        ui.label("Attack");
        num_f64(ui, &mut c.attack_ms, 0.01..=2000.0, 0.5, " ms");
        ui.end_row();
        ui.label("Release");
        num_f64(ui, &mut c.release_ms, 1.0..=9000.0, 1.0, " ms");
        ui.end_row();
    });
}

pub fn range_ui(ui: &mut Ui, v: &mut bool) {
    ui.label("Colour range");
    ui.vertical(|ui| {
        ui.checkbox(v, "Convert full-range to limited-range BT.709 and tag it");
        hint(ui, "Use for full-range PC/capture sources. Skip if the source is already limited/video range (avoids grey blacks).");
    });
    ui.end_row();
}

pub fn faststart_ui(ui: &mut Ui, v: &mut bool, enabled: bool) {
    ui.label("Faststart");
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
