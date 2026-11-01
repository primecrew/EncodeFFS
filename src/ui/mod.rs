//! The egui application: menu, mode list, mode form, preview dialog, queue + log.

pub mod common;
pub mod modes;
pub mod presets;
pub mod updater;

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use ffenc::ffmpeg::locate::{self, FfPaths};
use ffenc::ffmpeg::runner::{full_args, JobStatus, Queue};
use ffenc::ffmpeg::version::{self, compare, FfmpegCheck, Verdict};
use ffenc::presets::{self as pstore, Preset};
use ffenc::settings::{self, Settings};
use ffenc::updater::{install, AvailableBuild};
use ffenc::util::{display_command, fmt_hms, now_unix};

use common::{hint, Bg};
use modes::{kinds, new_mode, Ctx, Plan};
use presets::{Action, Dialog, PresetUi};
use updater::{UpdaterEvent, UpdaterUi};

struct Preview {
    plan: Plan,
    program: PathBuf,
    text: String,
    overwrite: Vec<PathBuf>,
}

/// First-run / "FFmpeg not found" dialog state.
struct LocateDlg {
    path: String,
    first_run: bool,
}

pub struct App {
    settings: Settings,
    pre: PresetUi,
    presets_path: PathBuf,
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
    locate: Option<LocateDlg>,
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
        let first_run = !settings::settings_path().exists();
        let mut settings = Settings::load();
        let ctx = cc.egui_ctx.clone();
        let queue = Queue::new(Some(settings::logs_dir().join("FFmpegEncoder.log")), move || ctx.request_repaint());
        queue.log(format!("{} {} started. Settings: {}", ffenc::APP_NAME, ffenc::APP_VERSION, settings::settings_path().display()));
        let presets_path = pstore::presets_path();
        let had_file = presets_path.exists();
        let (mut list, warning) = pstore::load_from(&presets_path);
        if let Some(w) = warning {
            queue.log(w);
        }
        let known: Vec<&str> = kinds().into_iter().map(|k| k.0).collect();
        list.retain(|p| {
            let ok = known.contains(&p.kind.as_str());
            if !ok {
                queue.log(format!("Preset “{}” has an unknown type “{}” and was ignored.", p.name, p.kind));
            }
            ok
        });
        if list.is_empty() {
            list = pstore::factory_all();
        }
        if !had_file {
            migrate_legacy(&mut list, &mut settings);
        }
        let current = list.iter().position(|p| p.id == settings.last_preset).unwrap_or(0);
        let mut app = App {
            settings,
            pre: PresetUi { list, current, editors: Default::default(), dialog: None },
            presets_path,
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
            locate: None,
            startup_check: None,
            update_banner: None,
            rollback_confirm: false,
        };
        app.refresh_ffmpeg(&cc.egui_ctx);
        if !had_file {
            app.save_presets();
        }
        if first_run || app.ff.is_none() {
            app.open_locate(first_run);
        }
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
        self.settings.drafts = self.pre.drafts();
        self.settings.last_preset = self.pre.cur().id.clone();
        if let Err(e) = self.settings.save() {
            self.queue.log(format!("Could not save settings: {e}"));
        }
    }

    fn save_presets(&mut self) {
        if let Err(e) = pstore::save_to(&self.presets_path, &self.pre.list) {
            self.error = Some(e);
        }
    }

    fn open_locate(&mut self, first_run: bool) {
        let cur = self.settings.ffmpeg_bin_path.trim();
        let path = if cur.is_empty() { Settings::default().ffmpeg_bin_path } else { cur.to_string() };
        self.locate = Some(LocateDlg { path, first_run });
    }

    fn import_presets(&mut self, ctx: &egui::Context) {
        let Some(f) = rfd::FileDialog::new().add_filter("Presets (JSON)", &["json"]).pick_file() else { return };
        let res = std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| pstore::parse(&t));
        match res {
            Err(e) => self.error = Some(format!("Could not import {}: {e}", f.display())),
            Ok(list) => {
                let known: Vec<&str> = kinds().into_iter().map(|k| k.0).collect();
                let (new, skipped) = pstore::prepare_import(&self.pre.list, list, &known);
                let n = new.len();
                let first = self.pre.list.len();
                self.pre.list.extend(new);
                if n > 0 {
                    let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
                    let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
                    self.pre.select(first, &cx);
                }
                self.save_presets();
                let mut msg = format!("Imported {n} preset(s) from {}.", f.display());
                if !skipped.is_empty() {
                    msg.push_str(&format!("\nSkipped: {}", skipped.join(", ")));
                }
                self.queue.log(msg.clone());
                self.info = Some(("Import presets".into(), msg));
            }
        }
    }

    fn export_presets(&mut self, all: bool) {
        let list: Vec<Preset> = if all { (0..self.pre.list.len()).map(|i| self.pre.export_copy(i, &self.settings)).collect() } else { vec![self.pre.export_copy(self.pre.current, &self.settings)] };
        let name = if all { "FFmpegEncoder presets.json".to_string() } else { format!("{}.preset.json", ffenc::util::safe_name(&list[0].name)) };
        let Some(f) = rfd::FileDialog::new().add_filter("Presets (JSON)", &["json"]).set_file_name(name).save_file() else { return };
        match std::fs::write(&f, pstore::to_json(&list)) {
            Ok(()) => self.queue.log(format!("Exported {} preset(s) to {}", list.len(), f.display())),
            Err(e) => self.error = Some(format!("Could not write {}: {e}", f.display())),
        }
    }

    fn start_preview(&mut self, ctx: &egui::Context) {
        let Some(ff) = self.ff.clone() else {
            self.error = Some("FFmpeg was not found. Set the folder in File › Settings or use Help › Check for FFmpeg updates to download it.".into());
            return;
        };
        let idx = self.pre.current;
        let needs = self.pre.editor(idx, &self.settings).mode.needs_encoders();
        if needs {
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
        match self.pre.editor(idx, &self.settings).mode.build(&cx) {
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
            ui.menu_button("Presets", |ui| {
                let modified = self.pre.modified(&self.pre.cur().id.clone());
                if ui.button("New preset…").clicked() {
                    self.pre.dialog = Some(Dialog::New { name: String::new(), from: presets::NewFrom::Copy(self.pre.cur().id.clone()) });
                    ui.close_menu();
                }
                if ui.add_enabled(modified, egui::Button::new("Save preset   (Ctrl+S)")).clicked() {
                    self.pre.save_current(&self.settings);
                    self.save_presets();
                    ui.close_menu();
                }
                if ui.button("Save as new preset…").clicked() {
                    self.pre.dialog = Some(Dialog::SaveAs { name: format!("{} copy", self.pre.cur().name) });
                    ui.close_menu();
                }
                if ui.add_enabled(modified, egui::Button::new("Revert unsaved changes")).clicked() {
                    self.pre.revert_current(&self.settings);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Duplicate").clicked() {
                    self.pre.dialog = Some(Dialog::New { name: String::new(), from: presets::NewFrom::Copy(self.pre.cur().id.clone()) });
                    ui.close_menu();
                }
                if ui.button("Rename…").clicked() {
                    self.pre.dialog = Some(Dialog::Rename { id: self.pre.cur().id.clone(), name: self.pre.cur().name.clone() });
                    ui.close_menu();
                }
                if ui.add_enabled(self.pre.cur().builtin.is_some(), egui::Button::new("Reset to factory defaults…")).clicked() {
                    self.pre.dialog = Some(Dialog::Reset { id: self.pre.cur().id.clone() });
                    ui.close_menu();
                }
                if ui.button("Delete…").clicked() {
                    self.pre.dialog = Some(Dialog::Delete { id: self.pre.cur().id.clone() });
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Import presets…").clicked() {
                    ui.close_menu();
                    self.import_presets(ctx);
                }
                if ui.button("Export this preset…").clicked() {
                    ui.close_menu();
                    self.export_presets(false);
                }
                if ui.button("Export all presets…").clicked() {
                    ui.close_menu();
                    self.export_presets(true);
                }
                let missing = pstore::missing_builtins(&self.pre.list);
                if ui.add_enabled(!missing.is_empty(), egui::Button::new(format!("Restore missing built-in presets ({})", missing.len()))).clicked() {
                    self.pre.list.extend(missing);
                    self.save_presets();
                    ui.close_menu();
                }
                if ui.button("Open presets file").clicked() {
                    self.save_presets();
                    open_path(&self.presets_path);
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Locate FFmpeg folder…").clicked() {
                    self.open_locate(false);
                    ui.close_menu();
                }
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
            egui::Window::new(format!("Preview – {}", self.pre.list[self.pre.current].name)).collapsible(false).resizable(true).default_size([820.0, 420.0]).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
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

        // first run / FFmpeg not found
        let invalid = self.ff.is_none() || matches!(self.ff_check, Some(Err(_)));
        if self.locate.is_none() && invalid && !self.first_run_dismissed && !self.updater.open {
            self.open_locate(false);
        }
        if self.updater.open {
            // the updater replaces this dialog while it is open
        } else if let Some(mut dl) = self.locate.take() {
            let mut keep = true;
            let mut use_dir = None;
            let mut download = false;
            egui::Window::new(if dl.first_run { "Welcome – where is FFmpeg?" } else { "Locate FFmpeg" }).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(620.0);
                ui.label("Select the folder that contains ffmpeg.exe and ffprobe.exe.");
                if let (Some(f), true) = (&self.ff, !invalid) {
                    hint(ui, format!("Currently using {} ({}).", f.bin_dir.display(), f.source.label()));
                } else if let Some(Err(e)) = &self.ff_check {
                    ui.colored_label(Color32::LIGHT_RED, format!("The current FFmpeg doesn't run: {e}"));
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dl.path).desired_width(440.0));
                    if ui.button("Browse…").clicked() {
                        let mut d = rfd::FileDialog::new();
                        let cur = PathBuf::from(dl.path.trim());
                        if cur.is_dir() {
                            d = d.set_directory(&cur);
                        }
                        if let Some(p) = d.pick_folder() {
                            dl.path = p.to_string_lossy().into_owned();
                        }
                    }
                });
                let check = locate::check_bin_dir(Path::new(dl.path.trim()));
                match &check {
                    Ok(d) => {
                        ui.colored_label(Color32::from_rgb(120, 200, 120), format!("✔ Found {} and {} in {}", locate::exe("ffmpeg"), locate::exe("ffprobe"), d.display()));
                    }
                    Err(e) => {
                        ui.colored_label(Color32::LIGHT_RED, format!("✖ {e}"));
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(check.is_ok(), egui::Button::new(RichText::new("Use this folder").strong())).clicked() {
                        use_dir = check.clone().ok();
                    }
                    ui.separator();
                    download = ui.button("⬇ Download newest FFmpeg").clicked();
                    if ui.button("Skip for now").clicked() {
                        keep = false;
                        self.first_run_dismissed = true;
                    }
                });
                hint(ui, "Downloads go to the app-managed folder (nothing is installed system-wide). Change this later in File › Settings or Help › Locate FFmpeg folder.");
            });
            if let Some(d) = use_dir {
                self.settings.ffmpeg_bin_path = d.to_string_lossy().into_owned();
                self.save_settings();
                self.refresh_ffmpeg(ctx);
                self.first_run_dismissed = true;
                keep = false;
            }
            if download {
                self.updater.open_and_check(ctx, &self.settings);
                self.first_run_dismissed = true;
                keep = false;
            }
            if keep {
                self.locate = Some(dl);
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

        // presets
        let act = self.pre.dialogs(ctx);
        if !matches!(act, Action::None) {
            let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
            let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
            let r = self.pre.apply(act, &cx);
            if let Err(e) = r {
                self.error = Some(e);
            }
            self.save_presets();
            self.save_settings();
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
                common::section(ui, "Encoding defaults (used by built-in presets that are not customised, and by new blank presets)");
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
                    let drafts = std::mem::take(&mut ed.drafts);
                    ed = Settings { drafts, last_preset: ed.last_preset.clone(), ..Settings::default() };
                }
            });
        });
        if let Some(e) = err {
            self.error = Some(e);
        }
        if save {
            let ff_changed = ed.ffmpeg_bin_path != self.settings.ffmpeg_bin_path;
            self.settings = ed;
            self.pre.rebase(&self.settings);
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
            let idx = self.pre.current;
            self.pre.editor(idx, &self.settings).mode.on_drop(&dropped, &cx);
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            self.menu(ctx, ui);
            self.banners(ctx, ui);
        });

        // Ctrl+S saves the current preset
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) && self.pre.modified(&self.pre.cur().id.clone()) {
            self.pre.save_current(&self.settings);
            self.save_presets();
        }

        egui::SidePanel::left("presets").resizable(true).default_width(240.0).width_range(180.0..=420.0).show(ctx, |ui| {
            ui.add_space(4.0);
            let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
            let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
            if let Some(what) = self.pre.sidebar(ui, &cx) {
                if what == "moved" {
                    self.save_presets();
                }
                self.save_settings();
            }
        });

        egui::TopBottomPanel::bottom("queue").resizable(true).default_height(270.0).min_height(120.0).show(ctx, |ui| {
            self.queue_panel(ui);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let idx = self.pre.current;
            let (name, desc, kind) = {
                let p = self.pre.cur();
                (p.name.clone(), p.description.clone(), p.kind.clone())
            };
            let modified = self.pre.modified(&self.pre.cur().id.clone());
            let kind_label = kinds().into_iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or("?");
            let mut save = false;
            let mut save_as = false;
            let mut revert = false;
            ui.horizontal(|ui| {
                ui.heading(&name);
                ui.label(RichText::new(format!("[{kind_label}]")).small().weak());
                if modified {
                    ui.label(RichText::new("(unsaved changes)").small().color(Color32::from_rgb(230, 180, 60)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save_as = ui.small_button("Save as new…").on_hover_text("Save the form as a new preset").clicked();
                    revert = ui.add_enabled(modified, egui::Button::new("Revert").small()).on_hover_text("Back to the saved values (keeps the chosen files)").clicked();
                    save = ui.add_enabled(modified, egui::Button::new(RichText::new("Save preset").strong()).small()).on_hover_text("Store the current values in this preset (Ctrl+S)").clicked();
                });
            });
            if !desc.is_empty() {
                hint(ui, &desc);
            }
            ui.separator();
            if save {
                self.pre.save_current(&self.settings);
                self.save_presets();
            }
            if revert {
                self.pre.revert_current(&self.settings);
            }
            if save_as {
                self.pre.dialog = Some(Dialog::SaveAs { name: format!("{name} copy") });
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
                let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
                self.pre.editor(idx, &self.settings).mode.ui(ui, &cx);
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
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, format!("Drop files for “{}”", self.pre.cur().name), egui::FontId::proportional(24.0), Color32::WHITE);
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

/// 0.1.x stored the last-used values per mode in settings.modes. Turn those into
/// the built-in presets' saved values (and keep the files as drafts) once.
fn migrate_legacy(list: &mut [Preset], s: &mut Settings) {
    if s.modes.is_empty() {
        return;
    }
    for p in list.iter_mut() {
        let Some(old) = s.modes.get(&p.kind) else { continue };
        if p.builtin.is_none() || !p.values.is_null() {
            continue;
        }
        if let Some(mut m) = new_mode(&p.kind) {
            m.load(old, s);
            p.values = pstore::strip_run_fields(m.save(), m.run_fields());
            s.drafts.insert(p.id.clone(), m.save());
        }
    }
    if let Some(p) = list.iter().find(|p| p.kind == s.last_mode) {
        s.last_preset = p.id.clone();
    }
    s.modes.clear();
    s.last_mode.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_mode_values_become_presets() {
        let mut s = Settings::default();
        s.modes.insert("reencode".into(), json!({"input": "/v/a.mkv", "trim_start": 4.0, "out_dir": "/o"}));
        s.modes.insert("unknown".into(), json!({"x": 1}));
        s.last_mode = "reencode".into();
        let mut list = pstore::factory_all();
        migrate_legacy(&mut list, &mut s);
        let p = &list[0];
        assert_eq!(p.values["trim_start"], json!(4.0));
        assert_eq!(p.values["out_dir"], json!("/o"));
        assert!(p.values.get("input").is_none());
        assert_eq!(s.drafts["builtin-reencode"]["input"], json!("/v/a.mkv"));
        assert_eq!(s.last_preset, "builtin-reencode");
        assert!(s.modes.is_empty() && s.last_mode.is_empty());
        assert!(list[1].values.is_null());
        // the migrated form is not "modified"
        let e = presets::make_editor(p, &s, s.drafts.get(&p.id)).unwrap();
        assert!(!e.modified());
    }
}
