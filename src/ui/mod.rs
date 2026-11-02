//! The egui application: menu, mode list, mode form, preview dialog, queue + log.

pub mod chapter_creator;
pub mod common;
pub mod modes;
pub mod presets;
pub mod source;
pub mod updater;
pub mod theme;

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, RichText};
use ffenc::ffmpeg::locate::{self, FfPaths};
use ffenc::ffmpeg::runner::{full_args, JobSpec, JobStatus, PostAction, Queue};
use ffenc::output_guard::{self, Choice, Conflict, Outcome, Reason, Target, WhenExists};
use ffenc::ffmpeg::version::{self, compare, FfmpegCheck, Verdict};
use ffenc::presets::{self as pstore, Preset};
use ffenc::settings::{self, Settings};
use ffenc::updater::{install, AvailableBuild};
use ffenc::cmdline::{self, Shell};
use ffenc::util::{fmt_hms, now_unix};

use common::{copy_button, hint, Bg};
use modes::{kinds, new_mode, Ctx, Plan};
use presets::{Action, Dialog, PresetUi, SideEvent};
use updater::{UpdaterEvent, UpdaterUi};

struct Preview {
    /// Jobs as the preset built them (before overwrite protection).
    plan: Plan,
    program: PathBuf,
    /// Temp-file id per job (stable while the popup is open).
    ids: Vec<String>,
    /// target index -> job index (jobs that go through overwrite protection; remux has its own).
    tgt_job: Vec<usize>,
    targets: Vec<Target>,
    conflicts: Vec<Conflict>,
    /// One choice per conflict.
    choices: Vec<Choice>,
    policy: WhenExists,
    /// Final jobs (after choices): what Run queues and what is shown/copied.
    jobs: Vec<JobSpec>,
    /// Outcome per target.
    outcomes: Vec<Outcome>,
    /// Focus "Keep both" of the first conflict when the popup opens (Ask mode).
    focus_keep: bool,
    /// Which copy button was pressed last (None = "Copy all") and when.
    copied: Option<(Option<usize>, std::time::Instant)>,
    /// Put keyboard focus on Run when the popup opens (Enter/Space runs, Esc cancels).
    focus_run: bool,
}

/// The full command of one job as it will run, for the given terminal, followed by the
/// rename of the temporary output that the app does after success.
fn job_command(shell: Shell, program: &Path, j: &JobSpec) -> String {
    let mut s = cmdline::job_script(shell, program, &full_args(&j.args), &j.side_files);
    if let PostAction::Publish { temp, dest, overwrite } = &j.post {
        s.push_str(shell.newline());
        s.push_str(&cmdline::publish_line(shell, temp, dest, *overwrite));
    }
    s
}

/// Every job, one after another (CRLF separated), for "Copy all".
fn all_commands(shell: Shell, program: &Path, jobs: &[JobSpec]) -> String {
    jobs.iter().map(|j| job_command(shell, program, j)).collect::<Vec<_>>().join(shell.newline())
}

fn fmt_modified(t: Option<std::time::SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string(),
        None => "unknown date".into(),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

impl Preview {
    fn new(plan: Plan, program: PathBuf, policy: WhenExists) -> Self {
        let ids = plan.jobs.iter().map(|_| ffenc::util::unique_id()).collect();
        let mut tgt_job = vec![];
        let mut targets = vec![];
        for (i, j) in plan.jobs.iter().enumerate() {
            if let (Some(o), PostAction::None) = (&j.output, &j.post) {
                tgt_job.push(i);
                targets.push(Target { output: o.clone(), inputs: output_guard::job_inputs(j) });
            }
        }
        let conflicts = output_guard::find_conflicts(&targets, &output_guard::stat, output_guard::CASE_INSENSITIVE);
        let choices = conflicts.iter().map(|c| {
            let ch = policy.initial_choice();
            if ch == Choice::Overwrite && !c.can_overwrite() { Choice::KeepBoth } else { ch }
        }).collect();
        let focus_keep = policy == WhenExists::Ask && !conflicts.is_empty();
        let mut p = Preview { plan, program, ids, tgt_job, targets, conflicts, choices, policy, jobs: vec![], outcomes: vec![], focus_keep, copied: None, focus_run: !focus_keep };
        p.recompute();
        p
    }

    /// Apply the choices: final names, -n/-y, temp files.
    fn recompute(&mut self) {
        let choice_of = |t: usize| self.conflicts.iter().position(|c| c.job == t).map(|k| self.choices[k]).unwrap_or(Choice::KeepBoth);
        self.outcomes = output_guard::resolve(&self.targets, &choice_of, &|p| p.exists(), output_guard::CASE_INSENSITIVE);
        let mut jobs = vec![];
        for (i, j) in self.plan.jobs.iter().enumerate() {
            match self.tgt_job.iter().position(|&x| x == i) {
                None => jobs.push(j.clone()),
                Some(t) => match &self.outcomes[t] {
                    Outcome::Skip => {}
                    Outcome::Run { output, overwrite } => match output_guard::prepare(j.clone(), output, *overwrite, &self.ids[i]) {
                        Ok(s) => jobs.push(s),
                        // cannot happen for the built-in builders (output is always the last argument)
                        Err(_) => {}
                    },
                },
            }
        }
        self.jobs = jobs;
    }

    fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for k in &self.conflicts {
            match &self.outcomes[k.job] {
                Outcome::Skip => c.2 += 1,
                Outcome::Run { overwrite: true, .. } => c.1 += 1,
                Outcome::Run { .. } => c.0 += 1,
            }
        }
        c
    }

    /// Conflict list with per-file choices. Returns true when a choice changed.
    fn conflicts_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let n = self.conflicts.len();
        let pal = crate::ui::theme::c();
        ui.label(RichText::new(format!("⚠ {n} output file{} already exist{} or clash – choose what to do:", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" })).color(pal.warn).strong());
        if self.policy != WhenExists::Ask {
            hint(ui, format!("Pre-selected by File › Settings › When the output file exists: {}. You can still change each file here.", self.policy.label()));
        }
        let focus_keep = std::mem::replace(&mut self.focus_keep, false);
        for k in 0..n {
            let c = self.conflicts[k].clone();
            let name = file_name(&c.output);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&name).strong());
                hint(ui, c.output.display().to_string());
                for r in &c.reasons {
                    match r {
                        Reason::Exists(e) => ui.label(format!("Existing file: {}, modified {}", output_guard::fmt_size(e.size), fmt_modified(e.modified))),
                        Reason::SameAsInput => ui.colored_label(pal.err, "This is an input file of the job – it can never be overwritten."),
                        Reason::SameAsJob(j) => ui.label(format!("Job {} of this run already writes this file.", self.tgt_job[*j] + 1)),
                    };
                }
                let new_name = |p: &Self| match &p.outcomes[c.job] {
                    Outcome::Run { output, overwrite: false } if *output != c.output => Some(file_name(output)),
                    _ => None,
                };
                let keep_name = new_name(self).unwrap_or_else(|| "a numbered name".into());
                ui.horizontal_wrapped(|ui| {
                    let cur = self.choices[k];
                    let r = ui.radio(cur == Choice::KeepBoth, "Keep both");
                    let r = common::named(r, egui::WidgetType::RadioButton, &format!("Keep both: save the new file as {keep_name}"));
                    if focus_keep && k == 0 {
                        r.request_focus();
                    }
                    if r.clicked() && cur != Choice::KeepBoth {
                        self.choices[k] = Choice::KeepBoth;
                        changed = true;
                    }
                    let r = ui.add_enabled(c.can_overwrite(), egui::RadioButton::new(cur == Choice::Overwrite, "Overwrite"));
                    let why = if c.can_overwrite() { format!("Overwrite: replace the existing {name}") } else { format!("Overwrite is not possible for {name}") };
                    let r = common::named(r, egui::WidgetType::RadioButton, &why);
                    if r.clicked() && cur != Choice::Overwrite {
                        self.choices[k] = Choice::Overwrite;
                        changed = true;
                    }
                    let r = ui.radio(cur == Choice::Skip, "Skip this file");
                    let r = common::named(r, egui::WidgetType::RadioButton, &format!("Skip: don't create {name}"));
                    if r.clicked() && cur != Choice::Skip {
                        self.choices[k] = Choice::Skip;
                        changed = true;
                    }
                });
                let result = match &self.outcomes[c.job] {
                    Outcome::Skip => "Result: skipped; the existing file stays as it is.".to_string(),
                    Outcome::Run { overwrite: true, .. } => "Result: the existing file will be replaced when the new one is finished.".to_string(),
                    Outcome::Run { output, .. } => format!("Result: new file {}", file_name(output)),
                };
                ui.label(RichText::new(result).strong());
            });
        }
        if n > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label("Apply to all:");
                for (ch, label, name) in [
                    (Choice::KeepBoth, "Keep both", "Keep both for all files"),
                    (Choice::Overwrite, "Overwrite", "Overwrite all files that can be overwritten (the others keep both)"),
                    (Choice::Skip, "Skip", "Skip all conflicting files"),
                ] {
                    let r = common::named(ui.button(label), egui::WidgetType::Button, name);
                    if r.clicked() {
                        for (k, c) in self.conflicts.iter().enumerate() {
                            self.choices[k] = if ch == Choice::Overwrite && !c.can_overwrite() { Choice::KeepBoth } else { ch };
                        }
                        changed = true;
                    }
                }
            });
        }
        changed
    }
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
    /// Feedback after a file drop: (text, accepted, when).
    drop_note: Option<(String, bool, std::time::Instant)>,
    started: std::time::Instant,
    sim_drop_done: bool,
    /// Appearance currently applied to egui (None = not yet).
    applied: Option<ffenc::settings::Appearance>,
    bold_available: bool,
    /// Ctrl+wheel accumulator (ln of zoom).
    wheel_zoom: f32,
    /// Text-size slider value while dragging (applied on release).
    scale_edit: Option<f32>,
    show_help: bool,
    settings_edit: Option<Settings>,
    updater: UpdaterUi,
    show_about: bool,
    first_run_dismissed: bool,
    locate: Option<LocateDlg>,
    startup_check: Option<Bg<Result<AvailableBuild, String>>>,
    update_banner: Option<String>,
    rollback_confirm: bool,
    source: source::SourcePanel,
    /// Run requested while the form was still probing: retry until this deadline.
    pending_run: Option<std::time::Instant>,
}

/// Open a file or folder with the OS (Explorer on Windows).
pub fn open_path(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

const WIN_FONTS: &str = r"C:\Windows\Fonts";

/// Fonts: Segoe UI (Windows' UI font) as the main proportional font when present
/// – Segoe UI Bold (or Semibold) when bold text is on – then egui's defaults and
/// system fallbacks so unicode file names render. Returns whether a bold face was found.
fn install_fonts(ctx: &egui::Context, bold: bool) -> bool {
    let dir = std::env::var("FFENC_FONTS_DIR").unwrap_or_else(|_| WIN_FONTS.to_string());
    let read = |f: &str| std::fs::read(Path::new(&dir).join(f)).ok();
    let mut fonts = egui::FontDefinitions::default();
    let bold_face = read("segoeuib.ttf").or_else(|| read("seguisb.ttf"));
    let bold_found = bold_face.is_some();
    let main = if bold { bold_face.or_else(|| read("segoeui.ttf")) } else { read("segoeui.ttf") };
    if let Some(bytes) = main {
        fonts.font_data.insert("ui-main".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "ui-main".into());
    }
    for (i, f) in ["seguisym.ttf", "msyh.ttc", "YuGothM.ttc", "malgun.ttf"].iter().enumerate() {
        if let Some(bytes) = read(f) {
            let name = format!("sys{i}");
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push(name.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
    bold_found
}

/// Too narrow (in points, i.e. after text scaling) to show the Source panel next to the form.
fn source_panel_too_narrow(ctx: &egui::Context) -> bool {
    ctx.screen_rect().width() < 900.0
}

/// Keyboard shortcuts (also shown in Help › Keyboard shortcuts).
const SHORTCUTS: &[(&str, &str)] = &[
    ("F5", "Run selected preset (opens the confirmation with the full command)"),
    ("Enter / Space", "In the confirmation: Run (focused first) · elsewhere: press the focused button"),
    ("Esc", "Close the confirmation / dialog"),
    ("Ctrl+O", "Open file(s) for the selected preset (folder for folder presets)"),
    ("Ctrl+S", "Save preset"),
    ("Ctrl+F", "Search presets"),
    ("Ctrl + plus / Ctrl + minus", "Larger / smaller text and controls (also Ctrl + mouse wheel)"),
    ("Ctrl+0", "Text size 100 %"),
    ("Tab / Shift+Tab", "Next / previous control (focused control has a thick outline)"),
    ("Arrow keys", "Change sliders and numbers; move in lists and text"),
    ("F1", "This help"),
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let first_run = !settings::settings_path().exists();
        let mut settings = Settings::load();
        // appearance before the first frame, so panels are laid out at the right scale
        let bold_available = install_fonts(&cc.egui_ctx, settings.appearance.bold);
        theme::apply(&cc.egui_ctx, &settings.appearance);
        let applied = Some(settings.appearance.clone());
        let ctx = cc.egui_ctx.clone();
        let queue = Queue::new(Some(settings::logs_dir().join("FFmpegEncoder.log")), move || ctx.request_repaint());
        queue.log(format!("{} {} started. Settings: {}", ffenc::APP_NAME, ffenc::APP_VERSION, settings::settings_path().display()));
        queue.set_keep_temp(settings.keep_temp_files);
        let registry = settings::logs_dir().join("partial-outputs.txt");
        queue.set_partials_registry(Some(registry.clone()));
        if settings.keep_temp_files {
            queue.log("Keep temporary files for debugging is on: temporary files are kept and not cleaned up at startup.");
        } else {
            // chapter files left behind by a crash (older than 6 h, so a second instance's live jobs are safe)
            let stale = ffenc::cmd::chapters::cleanup_stale(&std::env::temp_dir(), Duration::from_secs(6 * 3600));
            if stale > 0 {
                queue.log(format!("Removed {stale} stale temporary chapter file(s)."));
            }
            // unfinished outputs of a crashed run (not touched for 30 min, so another instance's live encodes are safe)
            // (background thread: the folders may be slow or offline network drives)
            let dirs: Vec<PathBuf> = [settings.output_folder.as_str(), settings.input_folder.as_str()].iter().filter(|s| !s.is_empty()).map(PathBuf::from).collect();
            let (q, reg) = (queue.clone(), registry.clone());
            std::thread::spawn(move || {
                for p in output_guard::cleanup_stale(Some(&reg), &dirs, Duration::from_secs(30 * 60)) {
                    q.log(format!("Removed unfinished output left by an earlier run: {}", p.display()));
                }
            });
        }
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
            pre: PresetUi { list, current, editors: Default::default(), dialog: None, filter: String::new() },
            presets_path,
            queue,
            ff: None,
            ff_check: None,
            ff_task: None,
            preview: None,
            error: None,
            info: None,
            drop_note: None,
            started: std::time::Instant::now(),
            sim_drop_done: false,
            applied,
            bold_available,
            wheel_zoom: 0.0,
            scale_edit: None,
            show_help: false,
            settings_edit: None,
            updater: UpdaterUi::default(),
            show_about: false,
            first_run_dismissed: false,
            locate: None,
            startup_check: None,
            update_banner: None,
            rollback_confirm: false,
            source: Default::default(),
            pending_run: None,
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
        self.queue.set_keep_temp(self.settings.keep_temp_files);
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
        let pending = self.pending_run.take();
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
                self.preview = Some(Preview::new(plan, ff.ffmpeg.clone(), self.settings.when_exists));
            }
            Err(e) => {
                // double-click / Run right after switching presets: the form probes its input first
                if e.contains("still probing") || e.contains("Still scanning") {
                    let t0 = pending.unwrap_or_else(std::time::Instant::now);
                    if t0.elapsed() < Duration::from_secs(15) {
                        self.pending_run = Some(t0);
                        ctx.request_repaint_after(Duration::from_millis(150));
                        return;
                    }
                }
                self.error = Some(e);
            }
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
                let pinned = self.pre.cur().pinned;
                if ui.button(if pinned { "Remove from favorites" } else { "Pin to favorites" }).clicked() {
                    let i = self.pre.current;
                    self.pre.list[i].pinned = !pinned;
                    self.save_presets();
                    ui.close_menu();
                }
                if ui.button("Set folder…").clicked() {
                    self.pre.dialog = Some(Dialog::Category { id: self.pre.cur().id.clone(), category: self.pre.cur().category.clone() });
                    ui.close_menu();
                }
                if ui.button("Run selected preset   (F5)").clicked() {
                    ui.close_menu();
                    self.start_preview(ctx);
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
            ui.menu_button("View", |ui| {
                if ui.checkbox(&mut self.settings.show_source_panel, "Source info panel (ffprobe)").clicked() {
                    self.save_settings();
                    ui.close_menu();
                }
                if self.settings.show_source_panel && source_panel_too_narrow(ctx) {
                    hint(ui, "(hidden while the window is too narrow for it at this text size)");
                }
                ui.separator();
                ui.label(RichText::new("Theme").strong());
                for th in ffenc::settings::ThemeChoice::ALL {
                    if ui.radio_value(&mut self.settings.appearance.theme, th, th.label()).clicked() {
                        ui.close_menu();
                    }
                }
                ui.separator();
                let a = &mut self.settings.appearance;
                ui.label(RichText::new(format!("Text size: {:.0} %", a.scale * 100.0)).strong());
                if ui.button("Larger      Ctrl +").clicked() {
                    a.scale = ffenc::settings::step_scale(a.scale, true);
                }
                if ui.button("Smaller     Ctrl −").clicked() {
                    a.scale = ffenc::settings::step_scale(a.scale, false);
                }
                if ui.button("100 %        Ctrl+0").clicked() {
                    a.scale = 1.0;
                }
                ui.menu_button("Choose size", |ui| {
                    for s in ffenc::settings::SCALE_STEPS {
                        if ui.radio(a.scale == s, format!("{:.0} %", s * 100.0)).clicked() {
                            a.scale = s;
                            ui.close_menu();
                        }
                    }
                });
                ui.add_enabled(self.bold_available, egui::Checkbox::new(&mut a.bold, "Bold text"));
                ui.separator();
                if ui.button("Appearance settings…").clicked() {
                    self.settings_edit = Some(self.settings.clone());
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard shortcuts & accessibility   (F1)").clicked() {
                    self.show_help = true;
                    ui.close_menu();
                }
                ui.separator();
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
                        ui.colored_label(crate::ui::theme::c().err, "FFmpeg not found");
                    }
                    (Some(_), None) => {
                        ui.spinner();
                    }
                    (Some(f), Some(Ok(c))) => {
                        ui.label(RichText::new(format!("FFmpeg {}", c.version)).small()).on_hover_text(format!("{}\n({})", f.bin_dir.display(), f.source.label()));
                    }
                    (Some(_), Some(Err(e))) => {
                        ui.colored_label(crate::ui::theme::c().err, "FFmpeg error").on_hover_text(e);
                    }
                }
            });
        });
    }

    fn banners(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if let Some(Ok(c)) = &self.ff_check {
            if !c.has_libx264 || !c.has_aac {
                ui.colored_label(crate::ui::theme::c().err, "⚠ This FFmpeg build lacks libx264 and/or aac. Re-encode modes won't work. Use Help › Check for FFmpeg updates to install a GPL build.");
            }
        }
        let mut dismiss = false;
        if let Some(b) = &self.update_banner {
            ui.horizontal(|ui| {
                ui.colored_label(crate::ui::theme::c().warn, format!("ℹ {b}"));
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
            egui::ScrollArea::vertical().id_salt("queue").max_height(120.0).min_scrolled_height(24.0).auto_shrink([false, true]).show(ui, |ui| {
                if s.jobs.is_empty() {
                    hint(ui, "No jobs yet. Fill in a mode and press “Preview & run…”.");
                }
                egui::Grid::new("jobs").num_columns(4).striped(true).show(ui, |ui| {
                    for j in &s.jobs {
                        let (icon, color) = match &j.status {
                            JobStatus::Pending => ("⏳ pending", crate::ui::theme::c().muted),
                            JobStatus::Running => ("▶ running", crate::ui::theme::c().info),
                            JobStatus::Done => ("✔ done", crate::ui::theme::c().ok),
                            JobStatus::Failed(_) => ("✖ failed", crate::ui::theme::c().err),
                            JobStatus::Cancelled => ("■ cancelled", crate::ui::theme::c().muted),
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
        egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, false]).min_scrolled_height(40.0).show(ui, |ui| {
            for l in &s.log {
                let color = if l.starts_with("FAILED") || l.contains("Error") || l.contains("error") {
                    crate::ui::theme::c().err
                } else if l.starts_with("Processing complete") || l.starts_with("Queue finished") {
                    crate::ui::theme::c().ok
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
            common::fit(egui::Window::new("Problem").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.colored_label(crate::ui::theme::c().err, &e);
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
            common::fit(egui::Window::new(t).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
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
            let shell = &mut self.settings.copy_shell;
            let mut shell_changed = false;
            let last_copy = p.copied;
            let mut new_copy: Option<Option<usize>> = None;
            let focus_run = std::mem::replace(&mut p.focus_run, false);
            let copied_recently = |which: Option<usize>| last_copy.map(|(w, t)| w == which && t.elapsed() < Duration::from_millis(1500)).unwrap_or(false);
            common::fit(egui::Window::new(format!("Confirm – {}", self.pre.list[self.pre.current].name))
                .collapsible(false)
                .resizable(true)
                .default_size([900.0_f32.min(ctx.screen_rect().width() - 40.0), 520.0_f32.min(ctx.screen_rect().height() - 40.0)])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                , ctx, false).show(ctx, |ui| {
                    let n = p.jobs.len();
                    ui.horizontal(|ui| {
                        let head = if n == 0 { "Nothing to run – every job is skipped.".to_string() } else { format!("{n} job{} will be queued with these exact commands:", if n == 1 { "" } else { "s" }) };
                        ui.label(RichText::new(head).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::ComboBox::from_id_salt("copy_shell").selected_text(shell.label()).show_ui(ui, |ui| {
                                for s in Shell::ALL {
                                    shell_changed |= ui.selectable_value(shell, s, s.label()).changed();
                                }
                            });
                            ui.label("Command syntax:");
                        });
                    });
                    hint(ui, format!("ffmpeg: {}", p.program.display()));
                    for note in &p.plan.notes {
                        hint(ui, note);
                    }
                    ui.separator();
                    let bottom = 44.0;
                    let mut changed = false;
                    egui::ScrollArea::vertical().id_salt("confirm_jobs").auto_shrink([false, true]).max_height((ui.available_height() - bottom).max(160.0)).show(ui, |ui| {
                        if !p.conflicts.is_empty() {
                            changed = p.conflicts_ui(ui);
                            ui.separator();
                        }
                        for (i, j) in p.jobs.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("[{}] {}", i + 1, j.label)).strong());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if copied_recently(Some(i)) {
                                        ui.colored_label(crate::ui::theme::c().ok, "Copied");
                                    }
                                    let tip = if n == 1 { "Copy command to clipboard".to_string() } else { format!("Copy command {} to clipboard", i + 1) };
                                    if copy_button(ui, "Copy", &tip).clicked() {
                                        ui.ctx().copy_text(job_command(*shell, &p.program, j));
                                        new_copy = Some(Some(i));
                                    }
                                });
                            });
                            for w in &j.warnings {
                                if !w.starts_with("Chapters:") {
                                    hint(ui, w);
                                }
                            }
                            // read-only but selectable; wraps, never truncated
                            let cmd = job_command(*shell, &p.program, j);
                            ui.add(egui::TextEdit::multiline(&mut cmd.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(1));
                            if let PostAction::Publish { temp, dest, overwrite } = &j.post {
                                hint(ui, format!(
                                    "FFmpeg writes the temporary file {} first; when it succeeds the app renames it to {}{}. If it fails or is cancelled, only the temporary file is deleted.",
                                    file_name(temp), file_name(dest), if *overwrite { ", replacing the existing file" } else { " (never replacing an existing file)" }
                                ));
                            }
                            for (path, text) in &j.side_files {
                                let kind = if path.file_name().map(|f| f.to_string_lossy().starts_with(ffenc::cmd::join::LIST_PREFIX)).unwrap_or(false) { "concat list" } else { "chapter file" };
                                egui::CollapsingHeader::new(format!("Temporary {kind}: {}", path.display())).id_salt(("side", i, path.to_string_lossy().to_string())).default_open(n == 1).show(ui, |ui| {
                                    ui.add(egui::TextEdit::multiline(&mut text.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(2));
                                    hint(ui, match *shell {
                                        Shell::PowerShell => "Written by the app before the job and deleted afterwards. The copied PowerShell command writes it first, so it can be run on its own.",
                                        Shell::Cmd => "Written by the app before the job and deleted afterwards. To run the copied cmd command yourself, save this text (UTF-8) to that path first, or copy as PowerShell.",
                                    });
                                });
                            }
                            ui.add_space(6.0);
                        }
                    });
                    if changed {
                        p.recompute();
                    }
                    let n = p.jobs.len();
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        let (kb, ow, sk) = p.counts();
                        let summary = if p.conflicts.is_empty() { String::new() } else { format!(" – {kb} kept both, {ow} overwritten, {sk} skipped") };
                        let rb = ui.add_enabled(n > 0, egui::Button::new(RichText::new(if n == 1 { "▶  Run" } else { "▶  Run all" }).strong()).min_size(egui::vec2(110.0, 28.0)));
                        let rb = common::named(rb, egui::WidgetType::Button, &if n == 1 { format!("Run: add to the queue and start{summary}") } else { format!("Run all {n} jobs: add to the queue and start{summary}") });
                        if focus_run {
                            rb.request_focus();
                        }
                        if rb.clicked() {
                            run = true;
                        }
                        if ui.add(egui::Button::new("Cancel").min_size(egui::vec2(80.0, 28.0))).clicked() {
                            cancel = true;
                        }
                        ui.separator();
                        let (label, tip) = if n == 1 { ("Copy", "Copy command to clipboard") } else { ("Copy all", "Copy all commands to clipboard") };
                        if copy_button(ui, label, tip).clicked() {
                            ui.ctx().copy_text(all_commands(*shell, &p.program, &p.jobs));
                            new_copy = Some(None);
                        }
                        if copied_recently(None) {
                            ui.colored_label(crate::ui::theme::c().ok, "Copied");
                        }
                    });
                });
            if let Some(w) = new_copy {
                p.copied = Some((w, std::time::Instant::now()));
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
            if p.copied.map(|(_, t)| t.elapsed() < Duration::from_millis(1600)).unwrap_or(false) {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            if shell_changed {
                let _ = self.settings.save();
            }
        }
        if run {
            if let Some(p) = self.preview.take() {
                let skipped = p.plan.jobs.len().saturating_sub(p.jobs.len());
                if skipped > 0 {
                    self.queue.log(format!("Skipped {skipped} job(s) whose output already exists."));
                }
                if !p.jobs.is_empty() {
                    self.queue.enqueue(&p.program, p.jobs, p.plan.group, self.settings.keep_logs_on_success);
                }
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
            common::fit(egui::Window::new(if dl.first_run { "Welcome – where is FFmpeg?" } else { "Locate FFmpeg" }).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                ui.set_max_width(620.0);
                ui.label("Select the folder that contains ffmpeg.exe and ffprobe.exe.");
                hint(ui, "ffmpeg does the encoding; ffprobe (from the same folder) reads each input's duration, streams, interlacing and chapters.");
                if let (Some(f), true) = (&self.ff, !invalid) {
                    hint(ui, format!("Currently using {} ({}).", f.bin_dir.display(), f.source.label()));
                } else if let Some(Err(e)) = &self.ff_check {
                    ui.colored_label(crate::ui::theme::c().err, format!("The current FFmpeg doesn't run: {e}"));
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
                        ui.colored_label(crate::ui::theme::c().ok, format!("✔ Found {} and {} in {}", locate::exe("ffmpeg"), locate::exe("ffprobe"), d.display()));
                    }
                    Err(e) => {
                        ui.colored_label(crate::ui::theme::c().err, format!("✖ {e}"));
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
            common::fit(egui::Window::new("Roll back FFmpeg").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
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
            common::fit(egui::Window::new("About").open(&mut open).collapsible(false).resizable(true).default_width(560.0), ctx, true).show(ctx, |ui| {
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

    /// Theme / size shortcuts, then apply the appearance if it changed (live, saved).
    fn appearance_frame(&mut self, ctx: &egui::Context) {
        use ffenc::settings::step_scale;
        let a = &mut self.settings.appearance;
        let (up, down, reset) = ctx.input_mut(|i| {
            let up = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Plus) | i.consume_key(egui::Modifiers::COMMAND, egui::Key::Equals);
            let down = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Minus);
            let reset = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0);
            (up, down, reset)
        });
        if up {
            a.scale = step_scale(a.scale, true);
        }
        if down {
            a.scale = step_scale(a.scale, false);
        }
        if reset {
            a.scale = 1.0;
        }
        // Ctrl + mouse wheel: one step per notch-ish (trackpads send many small deltas)
        let zd = ctx.input(|i| i.zoom_delta());
        if zd != 1.0 && zd.is_finite() && zd > 0.0 {
            self.wheel_zoom += zd.ln();
            if self.wheel_zoom.abs() >= 0.09 {
                a.scale = step_scale(a.scale, self.wheel_zoom > 0.0);
                self.wheel_zoom = 0.0;
            }
        }
        *a = a.clone().sanitized();
        let a = a.clone();
        if self.applied.as_ref() != Some(&a) {
            let first = self.applied.is_none();
            if first || self.applied.as_ref().map(|p| p.bold) != Some(a.bold) {
                self.bold_available = install_fonts(ctx, a.bold);
            }
            theme::apply(ctx, &a);
            self.applied = Some(a.clone());
            if !first {
                self.save_settings();
            }
        }
        theme::begin_frame(ctx, &a);
    }

    /// Files from a drop or Ctrl+O into the selected preset.
    fn route_paths(&mut self, ctx: &egui::Context, dropped: Vec<PathBuf>) {
        let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
        let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
        let idx = self.pre.current;
        let ed = self.pre.editor(idx, &self.settings);
        let paths = if ed.mode.wants_folders() { dropped.clone() } else { ffenc::util::expand_dropped(&dropped) };
        let before = ed.mode.save();
        ed.mode.on_drop(&paths, &cx);
        let accepted = ed.mode.save() != before;
        let name = self.pre.cur().name.clone();
        self.drop_note = Some((
            if accepted {
                format!("Added to “{name}”")
            } else {
                format!("Nothing in the selection fits “{name}” – check the file type")
            },
            accepted,
            std::time::Instant::now(),
        ));
    }

    /// Ctrl+O: pick file(s) – or a folder for folder presets – for the selected preset.
    fn open_files(&mut self, ctx: &egui::Context) {
        let idx = self.pre.current;
        let folders = self.pre.editor(idx, &self.settings).mode.wants_folders();
        let mut d = rfd::FileDialog::new().set_title(format!("Open for “{}”", self.pre.cur().name));
        if Path::new(&self.settings.input_folder).is_dir() {
            d = d.set_directory(&self.settings.input_folder);
        }
        let picked = if folders { d.pick_folder().map(|p| vec![p]) } else { d.pick_files() };
        if let Some(v) = picked.filter(|v| !v.is_empty()) {
            self.route_paths(ctx, v);
        }
    }

    /// Theme, text size, monospace size, bold – edits `self.settings.appearance` live.
    fn appearance_ui(&mut self, ui: &mut egui::Ui) {
        use ffenc::settings::{ThemeChoice, MONO_MAX, MONO_MIN, SCALE_MAX, SCALE_MIN, SCALE_STEPS};
        let bold_ok = self.bold_available;
        egui::Grid::new("set_look").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            let a = &mut self.settings.appearance;
            ui.label("Theme");
            ui.horizontal_wrapped(|ui| {
                for th in ThemeChoice::ALL {
                    ui.radio_value(&mut a.theme, th, th.label());
                }
            });
            ui.end_row();
            ui.label("Text size");
            ui.vertical(|ui| {
                let mut v = self.scale_edit.unwrap_or(a.scale) * 100.0;
                let r = ui.add(egui::Slider::new(&mut v, SCALE_MIN * 100.0..=SCALE_MAX * 100.0).step_by(5.0).suffix(" %").text("whole interface"));
                let r = common::named(r, egui::WidgetType::Slider, "Text and interface size in percent");
                if r.dragged() {
                    // applied on release: rescaling under the mouse would move the slider
                    self.scale_edit = Some(v / 100.0);
                } else if r.changed() || r.drag_stopped() {
                    a.scale = v / 100.0;
                    self.scale_edit = None;
                }
                ui.horizontal_wrapped(|ui| {
                    for s in SCALE_STEPS {
                        if ui.selectable_label((a.scale - s).abs() < 0.001, format!("{:.0} %", s * 100.0)).clicked() {
                            a.scale = s;
                        }
                    }
                });
                hint(ui, "Scales all text, buttons, icons, lists and dialogs. Shortcuts: Ctrl + plus / minus, Ctrl+0, Ctrl + mouse wheel.");
            });
            ui.end_row();
            ui.label("Code / log text");
            ui.vertical(|ui| {
                let mut m = a.mono_scale * 100.0;
                let r = ui.add(egui::Slider::new(&mut m, MONO_MIN * 100.0..=MONO_MAX * 100.0).step_by(5.0).suffix(" %").text("of the text size"));
                if common::named(r, egui::WidgetType::Slider, "Monospace text size for commands and logs, in percent").changed() {
                    a.mono_scale = m / 100.0;
                }
                ui.label(RichText::new("ffmpeg -i \"input.mkv\" -c:v libx264 -crf 22 output.mp4").monospace());
            });
            ui.end_row();
            ui.label("Bold text");
            ui.vertical(|ui| {
                ui.add_enabled(bold_ok, egui::Checkbox::new(&mut a.bold, "Use a bold font for the interface"));
                if !bold_ok {
                    hint(ui, "Needs Segoe UI Bold (C:\\Windows\\Fonts\\segoeuib.ttf), which this system doesn't have.");
                }
            });
            ui.end_row();
        });
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut open = true;
        common::fit(egui::Window::new("Keyboard shortcuts & accessibility").open(&mut open).collapsible(false).resizable(true).default_width(620.0), ctx, true).show(ctx, |ui| {
            egui::Grid::new("keys").num_columns(2).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                for (k, what) in SHORTCUTS {
                    ui.label(RichText::new(*k).strong());
                    ui.label(*what);
                    ui.end_row();
                }
            });
            common::section(ui, "Accessibility");
            ui.label("• View › Theme: System, Dark, Light or High contrast. View › Text size or Ctrl + plus/minus: 75–300 %.");
            ui.label("• File › Settings › Appearance: text size slider, separate size for commands/logs, bold text.");
            ui.label("• Everything in the menus and forms can be reached with Tab; the focused control has a thick outline.");
            ui.label("• Screen readers (Narrator, NVDA) can read buttons, labels, check boxes and text fields through UI Automation. Some parts (the queue list, the drag-and-drop file list order, colours) are only partly announced.");
            ui.label("• Every action in a preset's right-click menu is also in the Presets menu.");
        });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_help = false;
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut ed) = self.settings_edit.take() else { return };
        let mut keep = true;
        let mut save = false;
        let mut err = None;
        common::fit(egui::Window::new("Settings").collapsible(false).resizable(true).default_width(640.0), ctx, false).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() - 150.0).max(120.0)).show(ui, |ui| {
                common::section(ui, "Appearance");
                hint(ui, "Applied and saved immediately (Save/Cancel below don't affect it).");
                self.appearance_ui(ui);
                common::section(ui, "Folders");
                egui::Grid::new("set_f").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    let d = ed.input_folder.clone();
                    common::path_row_nodrop(ui, "FFmpeg bin folder", &mut ed.ffmpeg_bin_path, common::Pick::Folder, &d);
                    ui.label("");
                    hint(ui, "Must contain ffmpeg.exe and ffprobe.exe (ffprobe is used to read source info).");
                    ui.end_row();
                    common::path_row_nodrop(ui, "Input folder", &mut ed.input_folder, common::Pick::Folder, &d);
                    common::path_row_nodrop(ui, "Output folder", &mut ed.output_folder, common::Pick::Folder, &d);
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
                    ui.label("Length tolerance");
                    ui.horizontal_wrapped(|ui| {
                        common::num_f64(ui, &mut ed.duration_tolerance_s, 0.0..=60.0, 0.1, " s");
                        common::hint(ui, "Audio & subtitles: added tracks this close to the video's length count as matching (no warning).");
                    });
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
                    ui.label("When the output file exists");
                    ui.vertical(|ui| {
                        for w in WhenExists::ALL {
                            ui.radio_value(&mut ed.when_exists, w, w.label());
                        }
                        hint(ui, "Ask shows each existing file in the run confirmation with Keep both / Overwrite / Skip. An input file is never overwritten.");
                    });
                    ui.end_row();
                    ui.label("");
                    ui.vertical(|ui| {
                        ui.checkbox(&mut ed.keep_temp_files, "Keep temporary files for debugging");
                        hint(ui, "Chapter and join-list files and unfinished outputs (*.ffenc-partial-*) are kept, their paths are written to the log, and they are not cleaned up at startup.");
                    });
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
            // appearance is edited live on self.settings, not on the dialog copy
            ed.appearance = self.settings.appearance.clone();
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

/// Test hook for drag & drop where the OS drag cannot be scripted (e.g. Wine):
/// `FFENC_SIMULATE_DROP="path1|path2"` hovers the paths from 3 s to 7 s after
/// start, then drops them once.
fn simulate_drop(start: std::time::Instant, done: &mut bool, raw: &mut egui::RawInput) {
    if *done {
        return;
    }
    let Ok(v) = std::env::var("FFENC_SIMULATE_DROP") else {
        *done = true;
        return;
    };
    let files: Vec<egui::HoveredFile> = v.split('|').filter(|s| !s.is_empty()).map(|s| egui::HoveredFile { path: Some(PathBuf::from(s)), mime: String::new() }).collect();
    let t = start.elapsed().as_secs_f32();
    if (3.0..7.0).contains(&t) {
        raw.hovered_files = files;
    } else if t >= 7.0 {
        raw.dropped_files = files.into_iter().map(|f| egui::DroppedFile { path: f.path, ..Default::default() }).collect();
        *done = true;
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let _ = ctx;
        simulate_drop(self.started, &mut self.sim_drop_done, raw);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.sim_drop_done {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.poll_background();

        self.appearance_frame(ctx);

        // drag & drop
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.route_paths(ctx, dropped);
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            self.menu(ctx, ui);
            self.banners(ctx, ui);
        });

        // F5 runs (previews) the selected preset
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) && self.preview.is_none() {
            self.start_preview(ctx);
        }
        // Ctrl+O open file(s), Ctrl+F search presets, F1 help
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) && self.preview.is_none() {
            self.open_files(ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new(presets::SEARCH_ID)));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.show_help = !self.show_help;
        }
        // Ctrl+S saves the current preset
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) && self.pre.modified(&self.pre.cur().id.clone()) {
            self.pre.save_current(&self.settings);
            self.save_presets();
        }

        let screen = ctx.screen_rect().size();
        egui::SidePanel::left("presets").resizable(true).default_width(240.0).width_range(150.0..=(screen.x * 0.45).max(200.0)).show(ctx, |ui| {
            ui.add_space(4.0);
            let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
            let cx = Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx };
            match self.pre.sidebar(ui, &cx) {
                None => {}
                Some(SideEvent::Selected) => self.save_settings(),
                Some(SideEvent::Changed) => {
                    self.save_presets();
                    self.save_settings();
                }
                Some(SideEvent::Run) => {
                    self.save_settings();
                    self.start_preview(ctx);
                }
                Some(SideEvent::Import) => self.import_presets(ctx),
                Some(SideEvent::Export { all }) => self.export_presets(all),
            }
        });

        // at large text sizes the window has few points of width: the form comes first
        if self.settings.show_source_panel && !source_panel_too_narrow(ctx) {
            egui::SidePanel::right("source").resizable(true).default_width(300.0).width_range(180.0..=(screen.x * 0.4).max(220.0)).show(ctx, |ui| {
                ui.add_space(4.0);
                let idx = self.pre.current;
                let input = presets::primary_input(self.pre.editor(idx, &self.settings));
                let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
                self.source.ui(ui, input, ffprobe.as_deref());
            });
        }

        egui::TopBottomPanel::bottom("queue").resizable(true).default_height(270.0_f32.min(screen.y * 0.3)).height_range(60.0..=(screen.y * 0.4).max(80.0)).show(ctx, |ui| {
            // never let the panel's content force it taller than its cap (large text sizes)
            egui::ScrollArea::vertical().id_salt("queue_panel_scroll").auto_shrink([false, false]).show(ui, |ui| {
                self.queue_panel(ui);
            });
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
            // wrapping rows: nothing overlaps at large text sizes / narrow windows
            ui.horizontal_wrapped(|ui| {
                ui.heading(&name);
                ui.label(RichText::new(format!("[{kind_label}]")).small().weak());
                if modified {
                    ui.label(RichText::new("(unsaved changes)").small().color(crate::ui::theme::c().unsaved));
                }
            });
            ui.horizontal_wrapped(|ui| {
                save = ui.add_enabled(modified, egui::Button::new(RichText::new("Save preset").strong()).small()).on_hover_text("Store the current values in this preset (Ctrl+S)").clicked();
                revert = ui.add_enabled(modified, egui::Button::new("Revert").small()).on_hover_text("Back to the saved values (keeps the chosen files)").clicked();
                save_as = ui.small_button("Save as new…").on_hover_text("Save the form as a new preset").clicked();
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
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
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

        if self.pending_run.is_some() && self.preview.is_none() && self.error.is_none() {
            self.start_preview(ctx);
        }

        // file hover: accent frame around the window + a banner; the fields that
        // accept files highlight themselves (common::path_row / the join list)
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let note = self.drop_note.as_ref().filter(|n| n.2.elapsed() < Duration::from_secs(3)).map(|n| (n.0.clone(), n.1));
        if hovering || note.is_some() {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            let rect = ctx.screen_rect();
            let pal = crate::ui::theme::c();
            let accent = pal.accent;
            let (text, color) = if hovering {
                painter.rect_stroke(rect.shrink(2.0), 6.0, egui::Stroke::new(4.0_f32, accent), egui::StrokeKind::Inside);
                let n = ctx.input(|i| i.raw.hovered_files.len());
                (format!("Drop {} for “{}” – highlighted fields accept files", if n > 1 { format!("{n} items") } else { "here".into() }, self.pre.cur().name), pal.pill_accent)
            } else {
                let (t, ok) = note.unwrap();
                (t, if ok { pal.pill_ok } else { pal.pill_warn })
            };
            // wraps at large text sizes instead of running off the window
            let galley = painter.layout(text, egui::FontId::proportional(17.0), pal.on_pill, (rect.width() - 80.0).max(100.0));
            let pill = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 30.0 + galley.size().y / 2.0), galley.size() + egui::vec2(28.0, 14.0));
            painter.rect_filled(pill, 14.0, color);
            if pal.high_contrast {
                painter.rect_stroke(pill, 14.0, egui::Stroke::new(2.0_f32, pal.accent), egui::StrokeKind::Inside);
            }
            painter.galley(pill.center() - galley.size() / 2.0, galley, pal.on_pill);
            ctx.request_repaint_after(Duration::from_millis(200));
        } else if self.drop_note.is_some() {
            self.drop_note = None;
        }

        self.dialogs(ctx);
        self.help_window(ctx);
        theme::focus_ring(ctx);

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

    #[test]
    fn copy_all_has_every_full_command() {
        use ffenc::ffmpeg::runner::JobSpec;
        let cmd = |a: &[&str]| ffenc::cmd::BuiltCommand { args: a.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let mut a = JobSpec::new("one", cmd(&["-i", r"C:\v\my clip.mkv", r"C:\out\a b.mp4"]), PathBuf::from("a"), PathBuf::from("a.log"), None);
        a.side_files = vec![(PathBuf::from(r"C:\T\ffenc_chapters_1.ffmeta"), ";FFMETADATA1\n".into())];
        let b = JobSpec::new("two", cmd(&["-i", "x.mkv", "y.mp4"]), PathBuf::from("b"), PathBuf::from("b.log"), None);
        let plan = Plan { jobs: vec![a, b], ..Default::default() };
        let prog = Path::new(r"C:\Program Files\ffmpeg\bin\ffmpeg.exe");
        let ps = all_commands(Shell::PowerShell, prog, &plan.jobs);
        let lines: Vec<&str> = ps.split("\r\n").collect();
        assert_eq!(lines.len(), 3, "{ps}");
        assert_eq!(lines[0], r#"[IO.File]::WriteAllText('C:\T\ffenc_chapters_1.ffmeta', (@(';FFMETADATA1') -join "`n") + "`n")"#);
        assert_eq!(lines[1], r"& 'C:\Program Files\ffmpeg\bin\ffmpeg.exe' -hide_banner -nostats -progress pipe:1 -i 'C:\v\my clip.mkv' 'C:\out\a b.mp4'");
        assert!(lines[2].ends_with("-i x.mkv y.mp4"));
        let c = all_commands(Shell::Cmd, prog, &plan.jobs);
        let cl: Vec<&str> = c.split("\r\n").collect();
        assert_eq!(cl.len(), 2, "cmd: commands only");
        assert_eq!(cl[0], r#""C:\Program Files\ffmpeg\bin\ffmpeg.exe" -hide_banner -nostats -progress pipe:1 -i "C:\v\my clip.mkv" "C:\out\a b.mp4""#);
        assert_eq!(job_command(Shell::Cmd, prog, &plan.jobs[1]), cl[1]);
    }

    #[test]
    fn preview_resolves_conflicts_and_commands() {
        let d = std::env::temp_dir().join(format!("ffenc_prev_{}", ffenc::util::unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        let input = d.join("in.mkv");
        std::fs::write(&input, "x").unwrap();
        let out = d.join("Out.mp4");
        std::fs::write(&out, "old").unwrap();
        let job = |o: &Path| {
            let args = vec!["-y".into(), "-i".into(), input.to_string_lossy().into_owned(), o.to_string_lossy().into_owned()];
            JobSpec::new("j", ffenc::cmd::BuiltCommand { args, ..Default::default() }, o.to_path_buf(), d.join("l.log"), None)
        };
        let fresh = d.join("new.mp4");
        let plan = Plan { jobs: vec![job(&out), job(&fresh), job(&fresh), job(&input)], ..Default::default() };
        let prog = Path::new("ffmpeg.exe");
        // Ask: keep both pre-selected, focus goes to the first "Keep both"
        let mut p = Preview::new(plan, prog.into(), WhenExists::Ask);
        assert!(p.focus_keep && !p.focus_run);
        assert_eq!(p.conflicts.iter().map(|c| c.job).collect::<Vec<_>>(), vec![0, 2, 3]);
        let outs: Vec<String> = p.jobs.iter().map(|j| file_name(j.output.as_ref().unwrap())).collect();
        assert_eq!(outs, ["Out (2).mp4", "new.mp4", "new (2).mp4", "in (2).mkv"]);
        assert!(p.jobs.iter().all(|j| j.args[0] == "-n"));
        let c = job_command(Shell::PowerShell, prog, &p.jobs[0]);
        assert!(c.contains(".ffenc-partial-") && c.contains("[IO.File]::Move(") && c.contains("Out (2).mp4"), "{c}");
        // overwrite only the real existing file; skip the rest
        p.choices = vec![Choice::Overwrite, Choice::Skip, Choice::Skip];
        p.recompute();
        assert_eq!(p.jobs.len(), 2);
        assert_eq!(p.jobs[0].args[0], "-y");
        assert_eq!(p.jobs[0].output.as_deref(), Some(out.as_path()));
        assert_eq!(p.counts(), (0, 1, 2));
        assert!(job_command(Shell::Cmd, prog, &p.jobs[0]).contains("move /Y"));
        // settings: always skip -> no focus on the conflict list, Run focused
        let plan = Plan { jobs: vec![job(&out)], ..Default::default() };
        let p = Preview::new(plan, prog.into(), WhenExists::Skip);
        assert!(p.jobs.is_empty() && p.focus_run && !p.focus_keep);
        // always overwrite: an input is still never overwritten
        let plan = Plan { jobs: vec![job(&input)], ..Default::default() };
        let p = Preview::new(plan, prog.into(), WhenExists::Overwrite);
        assert_eq!(p.choices, vec![Choice::KeepBoth]);
        assert_eq!(file_name(p.jobs[0].output.as_ref().unwrap()), "in (2).mkv");
        std::fs::remove_dir_all(&d).unwrap();
    }

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
