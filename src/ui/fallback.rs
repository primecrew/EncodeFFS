//! Prompt to download the app-managed FFmpeg when the user's copy lacks an encoder.
//! Installs only into the app-managed folder; the user's FFmpeg is never touched
//! and the FFmpeg folder setting is not changed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText};

use ffenc::cmd::codecs;
use ffenc::ffmpeg::locate;
use ffenc::settings::Settings;
use ffenc::updater::install::{self, Stage};
use ffenc::updater::{self, AvailableBuild};

use super::common::{hint, Bg};
use super::modes::Plan;

pub struct FallbackPrompt {
    pub short: String,
    pub plan: Plan,
    /// Conflicts to show after the download, if any.
    pub items: Vec<ffenc::cmd::conflicts::Conflict>,
    pub origin: Option<ffenc::job_history::Origin>,
    stage: StageUi,
}

enum StageUi {
    Ask,
    Checking(Bg<Result<AvailableBuild, String>>),
    Installing {
        task: Bg<Result<String, String>>,
        stage: Arc<Mutex<Option<Stage>>>,
        cancel: Arc<AtomicBool>,
    },
    Failed(String),
}

pub enum FallbackEvent {
    None,
    Ready { plan: Plan, program: PathBuf, items: Vec<ffenc::cmd::conflicts::Conflict>, origin: Option<ffenc::job_history::Origin> },
    Cancelled,
}

impl FallbackPrompt {
    pub fn ask(short: String, plan: Plan, items: Vec<ffenc::cmd::conflicts::Conflict>, origin: Option<ffenc::job_history::Origin>) -> Self {
        Self { short, plan, items, origin, stage: StageUi::Ask }
    }

    /// Managed ffmpeg.exe once a download has finished (or if it was already there).
    pub fn managed_program() -> PathBuf {
        locate::managed_bin().join(locate::exe("ffmpeg"))
    }

    pub fn show(&mut self, ctx: &egui::Context, settings: &Settings) -> FallbackEvent {
        // poll
        match &self.stage {
            StageUi::Checking(t) => {
                if let Some(r) = t.poll() {
                    match r {
                        Ok(b) => {
                            let target = locate::managed_bin();
                            let stage = Arc::new(Mutex::new(None));
                            let cancel = Arc::new(AtomicBool::new(false));
                            let st = stage.clone();
                            let c2 = ctx.clone();
                            let ca = cancel.clone();
                            let task = Bg::spawn(ctx, move || install::run_update(&b, &target, &|s| {
                                *st.lock().unwrap() = Some(s);
                                c2.request_repaint();
                            }, &ca));
                            self.stage = StageUi::Installing { task, stage, cancel };
                        }
                        Err(e) => self.stage = StageUi::Failed(e),
                    }
                }
            }
            StageUi::Installing { task, .. } => {
                if let Some(r) = task.poll() {
                    match r {
                        Ok(_) => {
                            let program = Self::managed_program();
                            if program.is_file() {
                                return FallbackEvent::Ready {
                                    plan: self.plan.clone(),
                                    program,
                                    items: self.items.clone(),
                                    origin: self.origin.clone(),
                                };
                            }
                            self.stage = StageUi::Failed("The download finished, but the app-managed FFmpeg was not found.".into());
                        }
                        Err(e) => self.stage = StageUi::Failed(e),
                    }
                }
            }
            _ => {}
        }

        let mut event = FallbackEvent::None;
        let open = true;
        crate::ui::common::fit(
            egui::Window::new("FFmpeg download")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
            ctx,
            false,
        )
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label(RichText::new(codecs::download_prompt(&self.short)));
            hint(ui, "The current FFmpeg build is downloaded into the app-managed folder. Your own FFmpeg is not changed.");
            match &self.stage {
                StageUi::Ask => {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("Download").strong())).clicked() {
                            let (src, ch) = (settings.update_source, settings.update_channel);
                            self.stage = StageUi::Checking(Bg::spawn(ctx, move || updater::check(src, ch)));
                        }
                        if ui.button("Cancel").clicked() {
                            event = FallbackEvent::Cancelled;
                        }
                    });
                }
                StageUi::Checking(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Looking up the current FFmpeg build…");
                    });
                    if ui.button("Cancel").clicked() {
                        event = FallbackEvent::Cancelled;
                    }
                }
                StageUi::Installing { stage, cancel, .. } => {
                    if let Some(st) = stage.lock().unwrap().clone() {
                        match st {
                            Stage::Downloading { done, total } => {
                                let f = total.map(|t| done as f32 / t.max(1) as f32).unwrap_or(0.0);
                                ui.add(egui::ProgressBar::new(f).text(format!(
                                    "Downloading {:.1} / {} MB",
                                    done as f64 / 1e6,
                                    total.map(|t| format!("{:.1}", t as f64 / 1e6)).unwrap_or_else(|| "?".into())
                                )));
                            }
                            other => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(format!("{other:?}…"));
                                });
                            }
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Starting download…");
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        cancel.store(true, Ordering::Relaxed);
                        event = FallbackEvent::Cancelled;
                    }
                }
                StageUi::Failed(e) => {
                    ui.colored_label(crate::ui::theme::c().err, e);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            self.stage = StageUi::Ask;
                        }
                        if ui.button("Cancel").clicked() {
                            event = FallbackEvent::Cancelled;
                        }
                    });
                }
            }
            let _ = open;
        });
        event
    }
}
