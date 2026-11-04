//! The "Encoding Finished" popup: shown when the queue becomes idle after at least one job
//! succeeded (one popup per run: a single job, a batch, or several queued jobs). Failed, cancelled
//! and skipped jobs keep their existing reporting in the Log. Can be turned off in Settings.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use ffenc::ffmpeg::runner::Job;
use ffenc::finished::{self, Finished, GoTo, Popup, Tracker};

use super::common::{self, hint};

#[derive(Default)]
pub struct FinishedUi {
    tracker: Tracker,
    popup: Option<Popup>,
    /// Put keyboard focus on the default button on the next frame (never on the same frame the
    /// popup appears, so a key already on its way can't press it).
    focus_next: u8,
}

/// Select the file in Explorer (`explorer.exe /select,"<path>"`), or open its folder when the
/// file is gone. The argument is passed verbatim (Explorer parses its own command line); paths with
/// a `"` can't be passed safely and open the folder instead, so nothing can break out of the quotes.
pub fn go_to_file(path: &Path) {
    match finished::go_to(path, path.is_file()) {
        GoTo::Select(arg) => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let exe = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("explorer.exe")).unwrap_or_else(|| PathBuf::from("explorer.exe"));
                let _ = std::process::Command::new(exe).raw_arg(arg).spawn();
            }
            #[cfg(not(windows))]
            {
                let _ = arg;
                if let Some(d) = path.parent() {
                    super::open_path(d);
                }
            }
        }
        GoTo::OpenFolder(d) => super::open_path(&d),
        GoTo::Nothing => {}
    }
}

fn name_of(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.rsplit(['\\', '/']).next().unwrap_or(&s).to_string()
}

fn folder_of(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.rfind(['\\', '/']) {
        Some(i) if i > 0 => PathBuf::from(&s[..i]),
        _ => p.to_path_buf(),
    }
}

fn details(f: &Finished) -> String {
    let mut v = vec![];
    if let Some(b) = f.size {
        v.push(finished::fmt_size(b));
    }
    if let Some(d) = f.elapsed {
        v.push(format!("took {}", finished::fmt_elapsed(d)));
    }
    v.join(" · ")
}

impl FinishedUi {
    /// Watch the queue (call every frame). `enabled` = Settings › Show Encoding Finished popup.
    pub fn watch(&mut self, ctx: &egui::Context, jobs: &[Job], enabled: bool) {
        let size_of = |p: &Path| std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len());
        if let Some(p) = self.tracker.update(jobs, &size_of) {
            if enabled {
                self.popup = Some(p);
                self.focus_next = 2;
                // flash the taskbar button when the app is in the background
                if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
                }
                ctx.request_repaint();
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.popup else { return };
        let mut close = false;
        // focus the default button, but not while the user is typing in a text field
        let typing = ctx.wants_keyboard_input();
        let mut want_focus = false;
        if self.focus_next > 0 {
            self.focus_next -= 1;
            want_focus = self.focus_next == 0 && !typing;
        }
        let single = p.items.len() == 1;
        let last = p.items.last().map(|f| f.path.clone()).unwrap_or_default();
        common::fit(egui::Window::new("Encoding Finished").id(egui::Id::new("encoding_finished")).collapsible(false).resizable(false).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]), ctx, false).show(ctx, |ui| {
            ui.set_max_width((ctx.screen_rect().width() - 60.0).clamp(240.0, 560.0));
            if single {
                let f = &p.items[0];
                ui.label(RichText::new(name_of(&f.path)).strong());
                hint(ui, format!("in {}", folder_of(&f.path).display()));
                let d = details(f);
                if !d.is_empty() {
                    ui.label(d);
                }
            } else {
                ui.label(RichText::new(format!("{} files finished", p.items.len())).strong());
                egui::ScrollArea::vertical().max_height((ctx.screen_rect().height() * 0.45).max(80.0)).show(ui, |ui| {
                    egui::Grid::new("finished_list").num_columns(2).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                        for f in &p.items {
                            ui.vertical(|ui| {
                                ui.label(name_of(&f.path)).on_hover_text(f.path.display().to_string());
                                let d = details(f);
                                if !d.is_empty() {
                                    hint(ui, d);
                                }
                            });
                            let b = common::named(ui.button("Go to file"), egui::WidgetType::Button, &format!("Go to file {}", name_of(&f.path)));
                            if b.clicked() {
                                go_to_file(&f.path);
                            }
                            ui.end_row();
                        }
                    });
                });
            }
            if p.not_done > 0 {
                ui.colored_label(crate::ui::theme::c().warn, format!("{} other job(s) failed or were cancelled – see the Log.", p.not_done));
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                let (label, tip) = if single { ("Go to file", "Show the file in Explorer") } else { ("Open folder", "Open the folder of the last file") };
                let main = ui.add(egui::Button::new(RichText::new(label).strong())).on_hover_text(tip);
                if want_focus {
                    main.request_focus();
                }
                if main.clicked() {
                    if single {
                        go_to_file(&last);
                    } else {
                        super::open_path(&folder_of(&last));
                    }
                }
                if single && ui.button("Open folder").on_hover_text("Open the folder in Explorer").clicked() {
                    super::open_path(&folder_of(&last));
                }
                if ui.button("Close").on_hover_text("Close (Esc)").clicked() {
                    close = true;
                }
            });
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.popup = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_folders_of_windows_paths() {
        assert_eq!(name_of(Path::new(r"C:\Vids\My clip ✓ (2).mp4")), "My clip ✓ (2).mp4");
        assert_eq!(folder_of(Path::new(r"C:\Vids\My clip ✓ (2).mp4")), PathBuf::from(r"C:\Vids"));
        assert_eq!(folder_of(Path::new("/tmp/a.mp4")), PathBuf::from("/tmp"));
    }
}
