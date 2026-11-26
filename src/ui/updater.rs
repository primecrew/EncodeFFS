//! Help > Check for FFmpeg updates… dialog (and the first-run download).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, RichText};
use ffenc::ffmpeg::locate::{managed_bin, FfPaths, FfSource};
use ffenc::ffmpeg::version::{compare, parse_version, Verdict};
use ffenc::settings::Settings;
use ffenc::updater::install::{self, Stage};
use ffenc::updater::{self, AvailableBuild, UpdateChannel, UpdateSource};

use super::common::{hint, Bg};

#[derive(Default)]
pub struct UpdaterUi {
    pub open: bool,
    source: UpdateSource,
    channel: UpdateChannel,
    check: Option<Bg<Result<AvailableBuild, String>>>,
    result: Option<Result<AvailableBuild, String>>,
    in_place: bool,
    confirm_in_place: bool,
    install: Option<Bg<Result<String, String>>>,
    install_target: Option<(PathBuf, bool)>,
    stage: Arc<Mutex<Option<Stage>>>,
    cancel: Arc<AtomicBool>,
    message: Option<Result<String, String>>,
}

/// Something the app must react to.
pub enum UpdaterEvent {
    None,
    /// Installed into `bin`; `managed` = app-managed copy.
    Installed { bin: PathBuf, managed: bool },
}

pub fn verdict_text(v: Verdict) -> (&'static str, Color32) {
    match v {
        Verdict::UpdateAvailable => ("Update available", crate::ui::theme::c().warn),
        Verdict::UpToDate => ("Up to date", crate::ui::theme::c().ok),
        Verdict::InstalledNewer => ("Installed build is newer", crate::ui::theme::c().ok),
        Verdict::Unknown => ("Can't compare (different channel/build type)", crate::ui::theme::c().muted),
    }
}

impl UpdaterUi {
    /// Open the dialog and start a check with the settings' source/channel.
    /// Manual checks are always allowed, even with "Turn off Update Check for FFMPEG" checked.
    pub fn open_and_check(&mut self, ctx: &egui::Context, s: &Settings) {
        debug_assert!(s.may_check_updates(ffenc::settings::CheckTrigger::Manual, 0, ffenc::settings::FfmpegInUse::OwnFolder));
        self.open = true;
        if self.install.is_none() {
            self.source = s.update_source;
            self.channel = s.update_channel;
            self.start_check(ctx);
        }
    }

    fn start_check(&mut self, ctx: &egui::Context) {
        let (src, ch) = (self.source, self.channel);
        self.result = None;
        self.message = None;
        self.check = Some(Bg::spawn(ctx, move || updater::check(src, ch)));
    }

    pub fn busy(&self) -> bool {
        self.install.is_some()
    }

    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings, ff: Option<&FfPaths>, ff_version: Option<&str>, queue_busy: bool) -> UpdaterEvent {
        let mut event = UpdaterEvent::None;
        if let Some(t) = &self.check {
            if let Some(r) = t.poll() {
                self.result = Some(r);
                self.check = None;
            }
        }
        if let Some(t) = &self.install {
            if let Some(r) = t.poll() {
                self.install = None;
                if r.is_ok() {
                    if let Some((bin, managed)) = self.install_target.take() {
                        event = UpdaterEvent::Installed { bin, managed };
                    }
                }
                self.message = Some(r);
            }
        }
        if !self.open {
            return event;
        }
        let mut open = self.open;
        crate::ui::common::fit(egui::Window::new("FFmpeg updates").open(&mut open).collapsible(false).resizable(true).default_width(620.0), ctx, true).remember("updates", true).show(ctx, |ui| {
            if ff.is_some_and(|f| !f.is_app_managed()) {
                hint(ui, "You're using your own FFmpeg folder, so the app never checks for updates by itself. This check runs only because you asked; installing the newest build puts it in the app's own copy and leaves your folder as it is.");
            } else if settings.never_check_updates {
                hint(ui, "Automatic update checks are off (File › Settings › “Turn off Update Check for FFMPEG”). This check runs only because you asked for it.");
            }
            egui::Grid::new("upd").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                crate::ui::common::row_label(ui, "Installed");
                ui.vertical(|ui| {
                    match (ff, ff_version) {
                        (Some(f), Some(v)) => {
                            ui.label(RichText::new(v).strong());
                            hint(ui, format!("{} ({})", f.bin_dir.display(), f.source.label()));
                        }
                        (Some(f), None) => {
                            ui.label(format!("checking… {}", f.bin_dir.display()));
                        }
                        _ => {
                            ui.colored_label(crate::ui::theme::c().err, "FFmpeg not found");
                        }
                    }
                });
                ui.end_row();
                crate::ui::common::row_label(ui, "Source");
                let mut changed = false;
                ui.horizontal(|ui| {
                    for s in UpdateSource::ALL {
                        changed |= ui.radio_value(&mut self.source, s, s.label()).changed();
                    }
                });
                ui.end_row();
                crate::ui::common::row_label(ui, "Channel");
                ui.horizontal(|ui| {
                    for c in UpdateChannel::ALL {
                        changed |= ui.radio_value(&mut self.channel, c, c.label()).changed();
                    }
                });
                ui.end_row();
                if changed && self.install.is_none() {
                    settings.update_source = self.source;
                    settings.update_channel = self.channel;
                    let _ = settings.save();
                    self.start_check(ctx);
                }
                crate::ui::common::row_label(ui, "Newest");
                ui.vertical(|ui| {
                    if self.check.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("checking…");
                        });
                    }
                    match &self.result {
                        Some(Ok(b)) => {
                            ui.label(RichText::new(&b.label).strong());
                            let size = b.size.map(|s| format!("{:.0} MB · ", s as f64 / 1e6)).unwrap_or_default();
                            hint(ui, format!("{}{} · {}", size, b.asset_name, if b.sha256.is_some() { "SHA-256 published" } else { "no checksum published" }));
                            if let Some(v) = ff_version {
                                let (t, c) = verdict_text(compare(&parse_version(v), &b.version));
                                ui.colored_label(c, t);
                            }
                            ui.horizontal(|ui| {
                                ui.hyperlink_to("Release notes / changes", &b.notes_url);
                                ui.hyperlink_to("Build page", &b.page_url);
                            });
                        }
                        Some(Err(e)) => {
                            ui.colored_label(crate::ui::theme::c().err, e);
                        }
                        None => {}
                    }
                });
                ui.end_row();
            });
            ui.separator();

            // ---- target selection
            let configured = PathBuf::from(settings.ffmpeg_bin_path.trim());
            let custom_active = ff.map(|f| f.source != FfSource::Managed && f.source != FfSource::SystemPath && f.bin_dir != managed_bin()).unwrap_or(false);
            let managed = managed_bin();
            ui.label(RichText::new("Install to").strong());
            // never write into the user's own FFmpeg installation: updates only go to the app-managed copy
            self.in_place = false;
            ui.label(format!("App-managed copy: {}", managed.display()));
            if custom_active {
                let cur = ff.map(|f| f.bin_dir.clone()).unwrap_or(configured);
                hint(ui, format!("Your own FFmpeg in {} is never changed. After installing, the app uses its own copy (App's FFmpeg) and remembers your folder: pick My FFmpeg in Settings › FFmpeg to use, or in the FFmpeg menu at the top right, to switch back.", cur.display()));
            }
            ui.separator();

            // ---- action row
            let target = managed_bin();
            if let Some(st) = self.stage.lock().unwrap().clone().filter(|_| self.install.is_some()) {
                match st {
                    Stage::Downloading { done, total } => {
                        let f = total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.0);
                        ui.add(egui::ProgressBar::new(f).text(format!("Downloading {:.1} / {} MB", done as f64 / 1e6, total.map(|t| format!("{:.1}", t as f64 / 1e6)).unwrap_or_else(|| "?".into()))));
                    }
                    other => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!("{other:?}…"));
                        });
                    }
                }
            }
            ui.horizontal(|ui| {
                let can = matches!(self.result, Some(Ok(_))) && self.install.is_none() && !queue_busy && (!self.in_place || self.confirm_in_place);
                if ui.add_enabled(can, egui::Button::new(RichText::new("⬇ Download & install").strong())).clicked() {
                    if let Some(Ok(b)) = &self.result {
                        if self.in_place && !install::can_write(&target) {
                            self.message = Some(Err(format!("{} is not writable. Use the app-managed copy instead.", target.display())));
                        } else {
                            let b = b.clone();
                            let t = target.clone();
                            let stage = self.stage.clone();
                            self.cancel.store(false, Ordering::Relaxed);
                            let cancel = self.cancel.clone();
                            let c2 = ctx.clone();
                            self.install_target = Some((target.clone(), !self.in_place));
                            self.message = None;
                            self.install = Some(Bg::spawn(ctx, move || {
                                install::run_update(&b, &t, &|s| {
                                    *stage.lock().unwrap() = Some(s);
                                    c2.request_repaint();
                                }, &cancel)
                            }));
                        }
                    }
                }
                if self.install.is_some() && ui.button("Cancel download").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                if ui.add_enabled(self.check.is_none() && self.install.is_none(), egui::Button::new("Check again")).clicked() {
                    self.start_check(ctx);
                }
            });
            if queue_busy {
                hint(ui, "Updating is disabled while jobs are running.");
            }
            hint(ui, "Help › Roll back FFmpeg goes back to the app's previous FFmpeg build. To use your own FFmpeg again, pick My FFmpeg in Settings › FFmpeg to use, or in the FFmpeg menu at the top right.");
            match &self.message {
                Some(Ok(m)) => {
                    ui.colored_label(crate::ui::theme::c().ok, m);
                }
                Some(Err(e)) => {
                    ui.colored_label(crate::ui::theme::c().err, e);
                }
                None => {}
            }
        });
        self.open = open || self.install.is_some();
        event
    }
}
