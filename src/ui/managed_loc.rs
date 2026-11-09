//! Settings › App-managed FFmpeg location (0.5.2): choose where the copy the app downloads
//! and updates lives, and move an existing copy there (copy, check it runs, delete the old one;
//! rolled back on failure). The rules and the move itself are in `ffenc::ffmpeg::managed`.

use std::path::PathBuf;

use eframe::egui::{self, RichText};
use ffenc::ffmpeg::locate::{self, exe};
use ffenc::ffmpeg::managed::{self, MoveReport};

use super::common::{self, hint, Bg};

/// A change waiting for the user's answer (move or not).
pub struct Pending {
    /// Value for the setting ("" = default).
    pub setting: String,
    pub new_root: PathBuf,
    pub old_root: PathBuf,
    /// The new folder already holds an FFmpeg (can't move into it; switching uses that copy).
    pub new_has_copy: bool,
}

/// What the app should do after a frame of this UI.
pub enum Outcome {
    /// Switch the setting (after a move, or without moving). `moved` = the copy came along.
    Switch { setting: String, old_root: PathBuf, moved: bool, note: Option<String> },
}

#[derive(Default)]
pub struct ManagedLocUi {
    /// Typed / picked folder.
    pub input: String,
    /// Last result or problem (ok?, text), shown under the controls.
    pub msg: Option<(bool, String)>,
    pub ask: Option<Pending>,
    task: Option<(Bg<Result<MoveReport, String>>, Pending)>,
}

pub const HELP: &str = "The FFmpeg the app downloads and keeps up to date (Help › Check for FFmpeg updates…), with its backup for Roll back. Not your own FFmpeg folder above. The default is in your user profile (%LOCALAPPDATA%); you can keep it on another drive or a network share instead.";

fn rules() -> managed::Rules {
    managed::Rules {
        current: managed::current_root().to_string_lossy().into_owned(),
        // the experimental edition's own data folder
        protected: locate::local_app_dir().parent().map(|p| ffenc::OTHER_EDITION_DATA_DIR_NAMES.iter().map(|n| p.join(n).to_string_lossy().into_owned()).collect()).unwrap_or_default(),
        system: managed::system_dirs(),
    }
}

impl ManagedLocUi {
    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// Check a requested location; on success either switch right away (nothing to move) or ask.
    /// `setting` = "" means back to the default.
    fn request(&mut self, setting: String) -> Option<Outcome> {
        let old_root = managed::current_root();
        let new_root = managed::resolve_root(&setting, &managed::default_root());
        if setting.is_empty() {
            if !managed::is_custom() {
                self.msg = Some((true, "The default location is already used.".into()));
                return None;
            }
        } else if let Err(e) = managed::validate_text(&setting, &rules()) {
            self.msg = Some((false, e));
            return None;
        }
        let has_old = managed::has_copy(&old_root);
        let need = managed::space_needed(has_old.then(|| managed::copy_size(&old_root)));
        if let Err(e) = managed::check_folder(&new_root, need) {
            self.msg = Some((false, e));
            return None;
        }
        let p = Pending { setting, new_has_copy: managed::has_copy(&new_root), new_root, old_root };
        if has_old {
            self.ask = Some(p);
            None
        } else {
            self.msg = None;
            Some(Outcome::Switch { setting: p.setting, old_root: p.old_root, moved: false, note: None })
        }
    }

    /// The controls inside the Settings dialog. `blocked` = jobs or an update are running.
    pub fn settings_ui(&mut self, ui: &mut egui::Ui, blocked: Option<&str>) -> Option<Outcome> {
        let mut out = None;
        let cur = managed::current_root();
        let custom = managed::is_custom();
        ui.horizontal_wrapped(|ui| {
            ui.label("Current:");
            ui.add(egui::Label::new(RichText::new(cur.join("bin").display().to_string()).monospace()).selectable(true));
            ui.label(if custom { "(your folder)" } else { "(default)" });
        });
        egui::Grid::new("set_mloc").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let start = cur.to_string_lossy().into_owned();
            common::path_row_nodrop(ui, "New location", &mut self.input, common::Pick::Folder, &start);
        });
        let busy = self.busy();
        let can = blocked.is_none() && !busy && self.ask.is_none();
        ui.horizontal_wrapped(|ui| {
            let change = ui.add_enabled(can && !self.input.trim().is_empty(), egui::Button::new("Change…")).on_hover_text("Check the folder above and use it for the app-managed FFmpeg (asks before moving anything).");
            if change.clicked() {
                out = self.request(self.input.trim().to_string());
            }
            let reset = ui.add_enabled(can && custom, egui::Button::new("Reset to default")).on_hover_text("Go back to the default location in your user profile.");
            if reset.clicked() {
                out = self.request(String::new());
            }
        });
        if let Some(b) = blocked {
            hint(ui, b);
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Moving the app-managed FFmpeg…");
            });
        }
        if let Some((ok, m)) = &self.msg {
            let c = if *ok { ui.visuals().text_color() } else { ui.visuals().error_fg_color };
            ui.label(RichText::new(m).color(c));
        }
        hint(ui, "Applies immediately (not with Save below). Default: %LOCALAPPDATA%\\FFmpegEncoder-Slideshow\\ffmpeg.");
        out
    }

    /// The "Move it?" question and the running move. Call every frame.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        if let Some((t, _)) = &self.task {
            if let Some(r) = t.poll() {
                let (_, p) = self.task.take().unwrap();
                return match r {
                    Ok(rep) => {
                        let mut m = format!("Moved the app-managed FFmpeg to {} ({} files, {} MB).", p.new_root.display(), rep.files, rep.bytes / 1_048_576);
                        if let Some(n) = &rep.note {
                            m.push(' ');
                            m.push_str(n);
                        }
                        self.msg = Some((true, m.clone()));
                        self.input.clear();
                        Some(Outcome::Switch { setting: p.setting, old_root: p.old_root, moved: true, note: Some(m) })
                    }
                    Err(e) => {
                        self.msg = Some((false, e));
                        None
                    }
                };
            }
        }
        let mut out = None;
        let Some(p) = &self.ask else { return None };
        let mut close = false;
        let mut start_move = false;
        // a modal: always above the Settings dialog, and nothing else can be clicked meanwhile
        egui::Modal::new(egui::Id::new("managed_loc_ask")).show(ctx, |ui| {
            ui.set_max_width((ctx.screen_rect().width() - 80.0).clamp(240.0, 640.0));
            ui.heading("Move the app-managed FFmpeg?");
            ui.add_space(4.0);
            ui.label(format!("The app-managed FFmpeg is in:\n{}", p.old_root.join("bin").display()));
            ui.label(format!("New location:\n{}", p.new_root.join("bin").display()));
            ui.add_space(6.0);
            if p.new_has_copy {
                ui.label(RichText::new("The new location already has an FFmpeg, so the current copy can't be moved there. Use the copy that's there, or cancel and choose an empty folder.").color(ui.visuals().warn_fg_color));
            } else {
                hint(ui, "Move: the files are copied, the copied FFmpeg is started once to check it works, then the old copy is deleted. If anything fails, the new folder is cleaned up and nothing changes.");
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if !p.new_has_copy {
                    let mv = ui.add(egui::Button::new(RichText::new("Move it there").strong()));
                    if !ui.memory(|m| m.focused().is_some()) {
                        mv.request_focus();
                    }
                    if mv.clicked() {
                        start_move = true;
                    }
                }
                let leave = if p.new_has_copy { "Use the copy that's there" } else { "Don't move (download a fresh copy later)" };
                if ui.button(leave).on_hover_text("Switch to the new location without moving. The old copy is left where it is (you can delete it yourself).").clicked() {
                    out = Some(Outcome::Switch { setting: p.setting.clone(), old_root: p.old_root.clone(), moved: false, note: None });
                    close = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        });
        if start_move {
            let p = self.ask.take().unwrap();
            let (old, new) = (p.old_root.clone(), p.new_root.clone());
            let t = Bg::spawn(ctx, move || {
                managed::move_copy(&old, &new, &|bin| ffenc::ffmpeg::version::check_ffmpeg(&bin.join(exe("ffmpeg"))).map(|_| ()))
            });
            self.msg = None;
            self.task = Some((t, p));
        } else if close {
            self.ask = None;
            if out.is_none() {
                self.msg = Some((true, "Not changed.".into()));
            }
        }
        out
    }
}
