//! File › Settings › “Encoding hardware” (1.1): GPU detection status, Use my GPU / CPU only,
//! Re-check, and the one-time “turn it on?” nudge. Detection runs on a background thread.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, Ui};
use ffenc::cmd::hwenc::{self, HwDetect, HwSettings};

use super::common::{warn, Bg};

pub const USE_GPU_TIP: &str = "Re-encodes use your graphics card (NVENC, Quick Sync or AMF) for H.264, HEVC and AV1. \
Much faster; files are a little bigger at the same visual quality. If the GPU fails on a file, that file is encoded again on the CPU. \
Copy/remux and Custom are not affected.";
pub const CPU_ONLY_TIP: &str = "Re-encodes use the CPU encoders (x264, x265, SVT-AV1): slower, but the best quality for the file size. This is the default.";
pub const RECHECK_TIP: &str = "Look again for GPU encoders in your FFmpeg (and the app's own copy) and run a 1-second test encode with each. Takes a few seconds; you can keep working.";
pub const WHERE: &str = "Change it any time in File › Settings… › Encoding hardware.";

/// Background detection + which FFmpeg executables were already checked automatically this run.
#[derive(Default)]
pub struct HwUi {
    task: Option<Bg<Vec<HwDetect>>>,
    auto_done: Vec<PathBuf>,
    /// Show the one-time nudge window.
    pub nudge: bool,
}

impl HwUi {
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// Check these executables now (Re-check).
    pub fn start(&mut self, ctx: &egui::Context, programs: Vec<PathBuf>) {
        if self.busy() || programs.is_empty() {
            return;
        }
        self.auto_done.extend(programs.iter().cloned());
        self.task = Some(Bg::spawn(ctx, move || programs.iter().map(|p| ffenc::ffmpeg::hwdetect::detect(p)).collect()));
    }

    /// First run (and a new FFmpeg): check executables that have no stored result yet.
    pub fn auto(&mut self, ctx: &egui::Context, hw: &HwSettings, programs: &[PathBuf]) {
        if self.busy() {
            return;
        }
        let todo: Vec<PathBuf> = programs
            .iter()
            .filter(|p| hw.for_program(&p.to_string_lossy()).is_none() && !self.auto_done.contains(p))
            .cloned()
            .collect();
        self.start(ctx, todo);
    }

    /// Store finished results into `hw`. Returns true when something arrived (the caller saves);
    /// sets `nudge` after the first successful detection while GPU encoding is still off.
    pub fn poll(&mut self, hw: &mut HwSettings) -> bool {
        let Some(r) = self.task.as_ref().and_then(|t| t.poll()) else { return false };
        self.task = None;
        let found = r.iter().any(|d| d.vendor().is_some());
        for d in r {
            hw.store(d);
        }
        if found && !hw.use_gpu && !hw.nudge_done {
            self.nudge = true;
        }
        true
    }
}

/// Larger, bold status text (the main facts of the section).
fn big(ui: &mut Ui, text: impl Into<String>) {
    let size = ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size * 1.15).unwrap_or(16.0);
    ui.label(RichText::new(text.into()).strong().size(size));
}

/// The Settings section. `use_gpu` is the dialog's copy; `live` holds the stored results.
/// Returns true when Re-check was clicked.
pub fn section_ui(ui: &mut Ui, use_gpu: &mut bool, live: &HwSettings, in_use: Option<&Path>, managed: Option<&Path>, busy: bool) -> bool {
    let det = in_use.and_then(|p| live.for_program(&p.to_string_lossy()));
    if busy {
        ui.horizontal(|ui| {
            ui.spinner();
            big(ui, "Checking your graphics card…");
        });
    } else {
        big(ui, hwenc::summary(det));
    }
    if let Some(n) = det.and_then(hwenc::ten_bit_note) {
        ui.label(n);
    }
    if let (Some(m), Some(i)) = (managed, in_use) {
        if m != i {
            if let Some(md) = live.for_program(&m.to_string_lossy()) {
                ui.label(format!("The app's own FFmpeg: {}", hwenc::summary(Some(md))));
            }
        }
    }
    ui.add_space(4.0);
    let size = ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size * 1.1).unwrap_or(15.0);
    ui.radio_value(use_gpu, true, RichText::new("Use my GPU").size(size)).on_hover_text(USE_GPU_TIP);
    ui.radio_value(use_gpu, false, RichText::new("CPU only (best quality per size)").size(size)).on_hover_text(CPU_ONLY_TIP);
    if *use_gpu && !busy && det.is_some_and(|d| d.vendor().is_none()) {
        warn(ui, "No GPU encoder works with this FFmpeg, so encoding stays on the CPU.");
    }
    let mut recheck = false;
    ui.horizontal(|ui| {
        let r = ui.add_enabled(!busy && in_use.is_some(), egui::Button::new(RichText::new("Re-check").size(size)).min_size(egui::vec2(110.0, 30.0)));
        if r.on_hover_text(RECHECK_TIP).on_disabled_hover_text("Checking… or FFmpeg was not found.").clicked() {
            recheck = true;
        }
    });
    ui.label("GPU encoding: much faster, slightly bigger files. CPU: slower, best quality per size. Copy/remux and Custom are not affected.");
    recheck
}

/// One-time offer after the first successful detection. Some(true) = turn it on, Some(false) = not now.
pub fn nudge_ui(ui: &mut Ui, det: &HwDetect) -> Option<bool> {
    let who = det.gpu_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} GPU", det.vendor().map(|v| v.name()).unwrap_or("Your")));
    let names: Vec<&str> = det.working_codecs().iter().map(|c| c.short()).collect();
    let list = match names.as_slice() {
        [a] => a.to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        _ => names.join(", "),
    };
    big(ui, format!("Your {who} can encode {list} much faster."));
    ui.label("Files come out slightly bigger at the same quality. Turn on GPU encoding?");
    ui.add_space(6.0);
    let mut out = None;
    ui.horizontal(|ui| {
        let size = ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size * 1.1).unwrap_or(15.0);
        let yes = ui.add(egui::Button::new(RichText::new("Use my GPU").strong().size(size)).min_size(egui::vec2(130.0, 32.0))).on_hover_text(USE_GPU_TIP);
        if yes.clicked() {
            out = Some(true);
        }
        if ui.add(egui::Button::new(RichText::new("Not now").size(size)).min_size(egui::vec2(110.0, 32.0))).on_hover_text(CPU_ONLY_TIP).clicked() {
            out = Some(false);
        }
    });
    ui.label(WHERE);
    out
}
