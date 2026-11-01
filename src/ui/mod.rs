//! The egui application: menu, mode list, mode form, preview dialog, queue + log.

pub mod common;
pub mod modes;
pub mod updater;

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use ffenc::ffmpeg::locate::{self, FfPaths};
use ffenc::ffmpeg::runner::{full_args, JobStatus, Queue};
use ffenc::ffmpeg::version::{self, compare, FfmpegCheck, Verdict};
use ffenc::settings::{self, Settings};
use ffenc::updater::{install, AvailableBuild};
use ffenc::util::{display_command, fmt_hms, now_unix};

use common::{hint, Bg};
use modes::{all_modes, Ctx, Mode, Plan};
use updater::{UpdaterEvent, UpdaterUi};

struct Preview {
    plan: Plan,
    program: PathBuf,
    text: String,
    overwrite: Vec<PathBuf>,
}

pub struct App {
    settings: Settings,
    modes: Vec<Box<dyn Mode>>,
    current: usize,
    queue: Queue,
    ff: Option<FfPaths>,
    ff_check: Option<Result<FfmpegCheck, String>>,
    ff_task: Option<Bg<Result<FfmpegCheck, String>>>,
    preview: Option<Preview>,
    error: Option<String>,
    info: Option<(String, String)>,
    settings_edit: Option<Settings>,
    updater: UpdaterUi,
    show_about: bool,
    first_run_dismissed: bool,
    startup_check: Option<Bg<Result<AvailableBuild, String>>>,
    update_banner: Option<String>,
    rollback_confirm: bool,
}

/// Open a file or folder with the OS (Explorer on Windows).
pub fn open_path(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

/// Add Windows system fonts as fallbacks so unicode file names render.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [r"C:\Windows\Fonts\segoeui.ttf", r"C:\Windows\Fonts\seguisym.ttf", r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\YuGothM.ttc", r"C:\Windows\Fonts\malgun.ttf"];
    for (i, path) in candidates.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("sys{i}");
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push(name.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        let settings = Settings::load();
        let ctx = cc.egui_ctx.clone();
        let queue = Queue::new(Some(settings::logs_dir().join("FFmpegEncoder.log")), move || ctx.request_repaint());
        queue.log(format!("{} {} started. Settings: {}", ffenc::APP_NAME, ffenc::APP_VERSION, settings::settings_path().display()));
        let mut modes = all_modes();
        for m in modes.iter_mut() {
            let v = settings.modes.get(m.key()).cloned().unwrap_or(serde_json::Value::Null);
            m.load(&v, &settings);
        }
        let current = modes.iter().position(|m| m.key() == settings.last_mode).unwrap_or(0);
        let mut app = App {
            settings,
            modes,
            current,
            queue,
            ff: None,
            ff_check: None,
            ff_task: None,
            preview: None,
            error: None,
            info: None,
            settings_edit: None,
            updater: UpdaterUi::default(),
            show_about: false,
            first_run_dismissed: false,
            startup_check: None,
            update_banner: None,
            rollback_confirm: false,
        };
        app.refresh_ffmpeg(&cc.egui_ctx);
        // Non-intrusive daily update check.
        if app.settings.auto_check_updates && now_unix().saturating_sub(app.settings.last_update_check) > 86_400 && app.ff.is_some() {
            let (s, c) = (app.settings.update_source, app.settings.update_channel);
            app.startup_check = Some(Bg::spawn(&cc.egui_ctx, move || ffenc::updater::check(s, c)));
        }
        app
    }

    fn refresh_ffmpeg(&mut self, ctx: &egui::Context) {
        self.ff = locate::locate(Path::new(self.settings.ffmpeg_bin_path.trim()));
        self.ff_check = None;
        self.ff_task = None;
        match &self.ff {
            Some(f) => {
                self.queue.log(format!("Using FFmpeg from {} ({})", f.bin_dir.display(), f.source.label()));
                let p = f.ffmpeg.clone();
                self.ff_task = Some(Bg::spawn(ctx, move || version::check_ffmpeg(&p)));
            }
            None => self.queue.log("FFmpeg not found."),
        }
    }

    fn ff_version(&self) -> Option<&str> {
        self.ff_check.as_ref().and_then(|r| r.as_ref().ok()).map(|c| c.version.as_str())
    }

    fn save_settings(&mut self) {
        for m in &self.modes {
            self.settings.modes.insert(m.key().to_string(), m.save());
        }
        self.settings.last_mode = self.modes[self.current].key().to_string();
        if let Err(e) = self.settings.save() {
            self.queue.log(format!("Could not save settings: {e}"));
        }
    }

    fn start_preview(&mut self, ctx: &egui::Context) {
        let Some(ff) = self.ff.clone() else {
            self.error = Some("FFmpeg was not found. Set the folder in File › Settings or use Help › Check for FFmpeg updates to download it.".into());
            return;
        };
        let mode = &self.modes[self.current];
        if mode.needs_encoders() {
            match &self.ff_check {
                Some(Ok(c)) if !c.has_libx264 || !c.has_aac => {
                    let missing: Vec<&str> = [(!c.has_libx264).then_some("libx264"), (!c.has_aac).then_some("aac")].into_iter().flatten().collect();
                    self.error = Some(format!("This FFmpeg build has no {} encoder. Install a GPL build via Help › Check for FFmpeg updates.", missing.join(" / ")));
                    return;
                }
                Some(Err(e)) => {
                    self.error = Some(format!("FFmpeg check failed: {e}"));
                    return;
                }
                _ => {}
            }
        }
        let cx = Ctx { settings: &self.settings, ffprobe: Some(&ff.ffprobe), egui: ctx };
        match self.modes[self.current].build(&cx) {
            Ok(plan) => {
                let mut text = String::new();
                let mut overwrite = vec![];
                for n in &plan.notes {
                    text.push_str(&format!("# {n}\n"));
                }
                for (i, j) in plan.jobs.iter().enumerate() {
                    text.push_str(&format!("\n# [{}] {}\n", i + 1, j.label));
                    for w in &j.warnings {
                        for l in w.lines() {
                            text.push_str(&format!("#   {l}\n"));
                        }
                    }
                    text.push_str(&display_command(&ff.ffmpeg, &full_args(&j.args)));
                    text.push('\n');
                    if let Some(o) = &j.output {
                        if o.exists() && matches!(j.post, ffenc::ffmpeg::runner::PostAction::None) {
                            overwrite.push(o.clone());
                        }
                    }
                }
                self.preview = Some(Preview { plan, program: ff.ffmpeg.clone(), text, overwrite });
            }
            Err(e) => self.error = Some(e),
        }
    }

    // ---------------------------------------------------------------- panels

    fn menu(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Settings…").clicked() {
                    self.settings_edit = Some(self.settings.clone());
                    ui.close_menu();
                }
                if ui.button("Open output folder").clicked() {
                    open_path(Path::new(&self.settings.output_folder));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Check for FFmpeg updates…").clicked() {
                    self.updater.open_and_check(ctx, &self.settings);
                    ui.close_menu();
                }
                let can_rb = self.ff.as_ref().map(|f| install::has_backup(&f.bin_dir)).unwrap_or(false);
                if ui.add_enabled(can_rb, egui::Button::new("Roll back FFmpeg to previous version…")).clicked() {
                    self.rollback_confirm = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Open logs folder").clicked() {
                    let d = settings::logs_dir();
                    let _ = std::fs::create_dir_all(&d);
                    open_path(&d);
                    ui.close_menu();
                }
                if ui.button("Open settings file").clicked() {
                    self.save_settings();
                    open_path(&settings::settings_path());
                    ui.close_menu();
                }
                if ui.button("Check for app updates…").clicked() {
                    self.info = Some(("App updates".into(), format!("Automatic app updates are not implemented yet.\nYou are running {} {}.", ffenc::APP_NAME, ffenc::APP_VERSION)));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("About").clicked() {
                    self.show_about = true;
                    ui.close_menu();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match (&self.ff, &self.ff_check) {
                    (None, _) => {
                        ui.colored_label(Color32::LIGHT_RED, "FFmpeg not found");
                    }
                    (Some(_), None) => {
                        ui.spinner();
                    }
                    (Some(f), Some(Ok(c))) => {
                        ui.label(RichText::new(format!("FFmpeg {}", c.version)).small()).on_hover_text(format!("{}\n({})", f.bin_dir.display(), f.source.label()));
                    }
                    (Some(_), Some(Err(e))) => {
                        ui.colored_label(Color32::LIGHT_RED, "FFmpeg error").on_hover_text(e);
                    }
                }
            });
        });
    }

    fn banners(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if let Some(Ok(c)) = &self.ff_check {
            if !c.has_libx264 || !c.has_aac {
                ui.colored_label(Color32::LIGHT_RED, "⚠ This FFmpeg build lacks libx264 and/or aac. Re-encode modes won't work. Use Help › Check for FFmpeg updates to install a GPL build.");
            }
        }
        let mut dismiss = false;
        if let Some(b) = &self.update_banner {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(230, 180, 60), format!("ℹ {b}"));
                if ui.small_button("Open updater").clicked() {
                    self.updater.open_and_check(ctx, &self.settings);
                    dismiss = true;
                }
                if ui.small_button("Dismiss").clicked() {
                    dismiss = true;
                }
            });
        }
        if dismiss {
            self.update_banner = None;
        }
    }

    fn queue_panel(&mut self, ui: &mut egui::Ui) {
        let shared = self.queue.shared.clone();
        let mut cancel_ids = vec![];
        ui.horizontal(|ui| {
            ui.label(RichText::new("Queue").strong());
            if ui.button("Cancel all").clicked() {
                self.queue.cancel_all();
            }
            if ui.button("Clear finished").clicked() {
                self.queue.clear_finished();
            }
        });
        {
            let s = shared.lock().unwrap();
            egui::ScrollArea::vertical().id_salt("queue").max_height(120.0).auto_shrink([false, true]).show(ui, |ui| {
                if s.jobs.is_empty() {
                    hint(ui, "No jobs yet. Fill in a mode and press “Preview & run…”.");
                }
                egui::Grid::new("jobs").num_columns(4).striped(true).show(ui, |ui| {
                    for j in &s.jobs {
                        let (icon, color) = match &j.status {
                            JobStatus::Pending => ("⏳ pending", Color32::GRAY),
                            JobStatus::Running => ("▶ running", Color32::LIGHT_BLUE),
                            JobStatus::Done => ("✔ done", Color32::from_rgb(120, 200, 120)),
                            JobStatus::Failed(_) => ("✖ failed", Color32::LIGHT_RED),
                            JobStatus::Cancelled => ("■ cancelled", Color32::GRAY),
                        };
                        ui.colored_label(color, icon);
                        let lbl = ui.label(&j.spec.label);
                        if let JobStatus::Failed(e) = &j.status {
                            lbl.on_hover_text(e);
                        }
                        let frac = j.progress.unwrap_or(0.0);
                        let mut text = match (j.spec.duration_s, j.status == JobStatus::Running) {
                            (Some(d), true) => format!("{} / {}", fmt_hms(j.pos_s), fmt_hms(d)),
                            (None, true) => fmt_hms(j.pos_s),
                            _ => String::new(),
                        };
                        if j.status == JobStatus::Running && !j.speed.is_empty() {
                            text.push_str(&format!("  ({})", j.speed));
                        }
                        if let Some(e) = j.elapsed {
                            text = format!("took {}", fmt_hms(e.as_secs_f64()));
                        }
                        ui.add(egui::ProgressBar::new(frac).desired_width(260.0).text(text));
                        if !j.status.finished() {
                            if ui.small_button("Cancel").clicked() {
                                cancel_ids.push(j.id);
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
            });
        }
        for id in cancel_ids {
            self.queue.cancel(id);
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Log").strong());
            if ui.small_button("Copy").clicked() {
                let text: Vec<String> = shared.lock().unwrap().log.iter().cloned().collect();
                ui.ctx().copy_text(text.join("\n"));
            }
            if ui.small_button("Clear").clicked() {
                shared.lock().unwrap().log.clear();
            }
        });
        let s = shared.lock().unwrap();
        egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for l in &s.log {
                let color = if l.starts_with("FAILED") || l.contains("Error") || l.contains("error") {
                    Color32::LIGHT_RED
                } else if l.starts_with("Processing complete") || l.starts_with("Queue finished") {
                    Color32::from_rgb(120, 200, 120)
                } else {
                    ui.visuals().text_color()
                };
                ui.label(RichText::new(l).monospace().size(11.5).color(color));
            }
        });
    }

    // --------------------------------------------------------------- dialogs

    fn dialogs(&mut self, ctx: &egui::Context) {
        // error / info
        if let Some(e) = self.error.clone() {
            let mut close = false;
            egui::Window::new("Problem").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.colored_label(Color32::LIGHT_RED, &e);
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
            if close {
                self.error = None;
            }
        }
        if let Some((t, m)) = self.info.clone() {
            let mut close = false;
            egui::Window::new(t).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(m);
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
            if close {
                self.info = None;
            }
        }

        // preview / confirm
        let mut run = false;
        let mut cancel = false;
        if let Some(p) = &mut self.preview {
            egui::Window::new("Preview – FFmpeg command").collapsible(false).resizable(true).default_size([820.0, 420.0]).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("{} job(s) will be queued:", p.plan.jobs.len()));
                for o in &p.overwrite {
                    ui.colored_label(Color32::from_rgb(230, 180, 60), format!("⚠ Output exists and will be overwritten: {}", o.display()));
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut p.text.as_str()).code_editor().desired_width(f32::INFINITY));
                });
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("▶ Run").strong()).clicked() {
                        run = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Copy commands").clicked() {
                        ui.ctx().copy_text(p.text.clone());
                    }
                });
            });
        }
        if run {
            if let Some(p) = self.preview.take() {
                self.queue.enqueue(&p.program, p.plan.jobs, p.plan.group, self.settings.keep_logs_on_success);
                self.save_settings();
            }
        }
        if cancel {
            self.preview = None;
        }

        // first run
        if self.ff.is_none() && !self.first_run_dismissed && !self.updater.open {
            let mut browse = false;
            let mut download = false;
            let mut later = false;
            egui::Window::new("FFmpeg not found").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label("FFmpeg Encoder needs ffmpeg.exe and ffprobe.exe. Looked in:");
                for (d, _) in locate::candidates(Path::new(self.settings.ffmpeg_bin_path.trim())) {
                    ui.label(RichText::new(format!("  • {}", d.display())).small());
                }
                ui.label(RichText::new("  • PATH").small());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    download = ui.button(RichText::new("⬇ Download newest FFmpeg").strong()).clicked();
                    browse = ui.button("Browse to existing folder…").clicked();
                    later = ui.button("Later").clicked();
                });
                hint(ui, "Downloads go to the app-managed folder; nothing is installed system-wide.");
            });
            if download {
                self.updater.open_and_check(ctx, &self.settings);
            }
            if later {
                self.first_run_dismissed = true;
            }
            if browse {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    let d = if locate::bin_dir_ok(&d) { Some(d) } else if locate::bin_dir_ok(&d.join("bin")) { Some(d.join("bin")) } else { None };
                    match d {
                        Some(d) => {
                            self.settings.ffmpeg_bin_path = d.to_string_lossy().into_owned();
                            self.save_settings();
                            self.refresh_ffmpeg(ctx);
                        }
                        None => self.error = Some("That folder doesn't contain ffmpeg.exe and ffprobe.exe.".into()),
                    }
                }
            }
        }

        // updater
        let qb = self.queue.is_busy();
        let ver = self.ff_version().map(|s| s.to_string());
        if let UpdaterEvent::Installed { bin, managed } = self.updater.show(ctx, &mut self.settings, self.ff.as_ref(), ver.as_deref(), qb) {
            if managed && Path::new(self.settings.ffmpeg_bin_path.trim()) != bin {
                self.queue.log(format!("FFmpeg folder setting changed from {} to the app-managed copy {}", self.settings.ffmpeg_bin_path, bin.display()));
                self.settings.ffmpeg_bin_path = bin.to_string_lossy().into_owned();
            }
            self.save_settings();
            self.refresh_ffmpeg(ctx);
            self.update_banner = None;
        }

        // rollback
        if self.rollback_confirm {
            let mut go = false;
            let mut no = false;
            let bin = self.ff.as_ref().map(|f| f.bin_dir.clone());
            egui::Window::new("Roll back FFmpeg").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                if let Some(b) = &bin {
                    ui.label(format!("Swap the binaries in\n{}\nwith the previous version in\n{}?", b.display(), install::backup_dir_for(b).display()));
                    hint(ui, "Running it again swaps back.");
                }
                ui.horizontal(|ui| {
                    go = ui.add_enabled(!self.queue.is_busy(), egui::Button::new("Roll back")).clicked();
                    no = ui.button("Cancel").clicked();
                });
            });
            if go {
                if let Some(b) = bin {
                    match install::rollback(&b) {
                        Ok(()) => self.queue.log(format!("Rolled back FFmpeg in {}", b.display())),
                        Err(e) => self.error = Some(e),
                    }
                    self.refresh_ffmpeg(ctx);
                }
                self.rollback_confirm = false;
            }
            if no {
                self.rollback_confirm = false;
            }
        }

        // about
        if self.show_about {
            let mut open = true;
            egui::Window::new("About").open(&mut open).collapsible(false).resizable(true).default_width(560.0).show(ctx, |ui| {
                ui.heading(format!("{} {}", ffenc::APP_NAME, ffenc::APP_VERSION));
                ui.label("GUI port of the FFmpeg processing PowerShell script v22.0.");
                ui.label(format!("Build: {} {}", std::env::consts::OS, std::env::consts::ARCH));
                ui.separator();
                match (&self.ff, &self.ff_check) {
                    (Some(f), Some(Ok(c))) => {
                        ui.label(format!("FFmpeg {}", c.version));
                        ui.label(format!("Location: {} ({})", f.bin_dir.display(), f.source.label()));
                        ui.label(format!("libx264: {}   aac: {}", if c.has_libx264 { "yes" } else { "NO" }, if c.has_aac { "yes" } else { "NO" }));
                        egui::CollapsingHeader::new("Build configuration").show(ui, |ui| {
                            ui.label(RichText::new(&c.configuration).small().monospace());
                        });
                    }
                    _ => {
                        ui.label("FFmpeg: not found");
                    }
                }
                ui.separator();
                ui.label(format!("Settings: {}", settings::settings_path().display()));
                ui.label(format!("Logs: {}", settings::logs_dir().display()));
                ui.label(format!("App-managed FFmpeg: {}", locate::managed_bin().display()));
            });
            self.show_about = open;
        }

        self.settings_dialog(ctx);
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ed) = self.settings_edit.take() else { return };
        let mut keep = true;
        let mut save = false;
        let mut err = None;
        egui::Window::new("Settings").collapsible(false).resizable(true).default_width(640.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                common::section(ui, "Folders");
                egui::Grid::new("set_f").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    let d = ed.input_folder.clone();
                    common::path_row(ui, "FFmpeg bin folder", &mut ed.ffmpeg_bin_path, common::Pick::Folder, &d);
                    common::path_row(ui, "Input folder", &mut ed.input_folder, common::Pick::Folder, &d);
                    common::path_row(ui, "Output folder", &mut ed.output_folder, common::Pick::Folder, &d);
                });
                hint(ui, "FFmpeg lookup order: this folder -> ffmpeg\\ next to the app -> app-managed copy -> PATH.");
                common::section(ui, "Encoding defaults (used when a mode has no remembered values)");
                egui::Grid::new("set_e").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    let mut v = ffenc::cmd::common::VideoEncode { crf: ed.crf, preset: ed.preset.clone(), tune: None };
                    common::video_encode_ui(ui, "set", &mut v, false);
                    ed.crf = v.crf;
                    ed.preset = v.preset;
                    common::bitrate_ui(ui, "Audio bitrate", &mut ed.audio_bitrate_k);
                    ui.label("Gain");
                    common::num_f64(ui, &mut ed.gain_db, -30.0..=30.0, 0.1, " dB");
                    ui.end_row();
                    ui.label("Trim start / end");
                    ui.horizontal(|ui| {
                        common::num_f64(ui, &mut ed.trim_start, 0.0..=1e7, 0.1, " s");
                        common::num_f64(ui, &mut ed.trim_end, 0.0..=1e7, 0.1, " s");
                    });
                    ui.end_row();
                    ui.label("Chapter interval");
                    common::num_f64(ui, &mut ed.chapter_interval_min, 0.1..=600.0, 0.1, " min");
                    ui.end_row();
                    ui.label("Commentary name");
                    ui.text_edit_singleline(&mut ed.commentary_name);
                    ui.end_row();
                    ui.label("Fallback sample rate");
                    common::num_u32(ui, &mut ed.fallback_sample_rate, 8000..=384000, " Hz");
                    ui.end_row();
                    ui.label("Advanced compressor");
                    common::compressor_params_ui(ui, "set_comp", &mut ed.compressor);
                    ui.end_row();
                });
                common::section(ui, "FFmpeg updates & logs");
                egui::Grid::new("set_u").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Source");
                    ui.horizontal(|ui| {
                        for s in ffenc::updater::UpdateSource::ALL {
                            ui.radio_value(&mut ed.update_source, s, s.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Channel");
                    ui.horizontal(|ui| {
                        for c in ffenc::updater::UpdateChannel::ALL {
                            ui.radio_value(&mut ed.update_channel, c, c.label());
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut ed.auto_check_updates, "Check for FFmpeg updates at startup (max once a day, never installs)");
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut ed.keep_logs_on_success, "Keep ffmpeg_error*.log files when a job succeeds");
                    ui.end_row();
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Save").strong()).clicked() {
                    match ed.validate() {
                        Ok(()) => save = true,
                        Err(e) => err = Some(e),
                    }
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
                if ui.button("Reset to script defaults").clicked() {
                    let modes = std::mem::take(&mut ed.modes);
                    ed = Settings { modes, ..Settings::default() };
                }
                if ui.button("Reset all remembered mode values").on_hover_text("Forms go back to the defaults above").clicked() {
                    ed.modes.clear();
                }
            });
        });
        if let Some(e) = err {
            self.error = Some(e);
        }
        if save {
            let reset_modes = ed.modes.is_empty();
            let ff_changed = ed.ffmpeg_bin_path != self.settings.ffmpeg_bin_path;
            self.settings = ed;
            if reset_modes {
                for m in self.modes.iter_mut() {
                    m.load(&serde_json::Value::Null, &self.settings);
                }
            }
            self.save_settings();
            if ff_changed || self.ff.is_none() {
                self.refresh_ffmpeg(ctx);
            }
        } else if keep {
            self.settings_edit = Some(ed);
        }
    }

    fn poll_background(&mut self) {
        if let Some(t) = &self.ff_task {
            if let Some(r) = t.poll() {
                match &r {
                    Ok(c) => {
                        self.queue.log(format!("FFmpeg {} – libx264: {}, aac: {}", c.version, c.has_libx264, c.has_aac));
                        if !c.has_libx264 || !c.has_aac {
                            self.queue.log("libx264 and/or AAC encoder not available in this FFmpeg build.");
                        }
                    }
                    Err(e) => self.queue.log(format!("FFmpeg check failed: {e}")),
                }
                self.ff_check = Some(r);
                self.ff_task = None;
            }
        }
        // Wait for the version before comparing.
        if self.ff_check.is_some() {
            if let Some(t) = &self.startup_check {
                if let Some(r) = t.poll() {
                    self.startup_check = None;
                    self.settings.last_update_check = now_unix();
                    let _ = self.settings.save();
                    if let (Ok(b), Some(v)) = (r, self.ff_version()) {
                        if compare(&version::parse_version(v), &b.version) == Verdict::UpdateAvailable {
                            self.update_banner = Some(format!("FFmpeg {} is available (installed: {v}).", b.label));
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background();

        // drag & drop
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
            let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
            self.modes[self.current].on_drop(&dropped, &cx);
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            self.menu(ctx, ui);
            self.banners(ctx, ui);
        });

        egui::SidePanel::left("modes").resizable(false).exact_width(230.0).show(ctx, |ui| {
            ui.add_space(4.0);
            for i in 0..self.modes.len() {
                let m = &self.modes[i];
                let r = ui.add_sized([220.0, 26.0], egui::SelectableLabel::new(i == self.current, m.title())).on_hover_text(m.description());
                if r.clicked() && i != self.current {
                    self.current = i;
                    self.save_settings();
                }
            }
        });

        egui::TopBottomPanel::bottom("queue").resizable(true).default_height(270.0).min_height(120.0).show(ctx, |ui| {
            self.queue_panel(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let (title, desc) = (self.modes[self.current].title(), self.modes[self.current].description());
            ui.horizontal(|ui| {
                ui.heading(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reset form").on_hover_text("Back to the defaults from Settings").clicked() {
                        let s = self.settings.clone();
                        self.modes[self.current].load(&serde_json::Value::Null, &s);
                    }
                });
            });
            hint(ui, desc);
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
                let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
                self.modes[self.current].ui(ui, &cx);
                ui.add_space(10.0);
                if ui.add_sized([180.0, 32.0], egui::Button::new(RichText::new("Preview & run…").strong())).clicked() {
                    self.start_preview(ctx);
                }
                ui.add_space(10.0);
            });
        });

        // file hover overlay
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            let rect = ctx.screen_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(160));
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("Drop files for “{}”", self.modes[self.current].title()), egui::FontId::proportional(24.0), Color32::WHITE);
        }

        self.dialogs(ctx);

        // keep progress bars moving while jobs run
        if self.queue.is_busy() || self.updater.busy() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.queue.cancel_all();
        self.save_settings();
        // Give a running ffmpeg a moment to stop cleanly.
        for _ in 0..30 {
            if !self.queue.is_busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
