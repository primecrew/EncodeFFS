//! "FFmpeg to use" (1.1): one-click choice between the user's own FFmpeg folder and the copy the
//! app downloads and keeps up to date. Shown at the top of the FFmpeg part of Settings and as the
//! drop-down behind the FFmpeg version in the menu bar. Both only *point* the app at a folder;
//! nothing is ever written to the user's own FFmpeg folder. The logic is in
//! `ffenc::settings::Settings::{use_own_ffmpeg, use_app_ffmpeg_at, set_own_ffmpeg}`.

use std::path::PathBuf;

use eframe::egui::{self, RichText};
use ffenc::ffmpeg::locate::shorten_path;
pub use ffenc::settings::FfmpegChoice;

use super::common::hint;

/// What one of the two choices looks like right now.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    /// bin folder; None = not set (mine) / not downloaded (app's).
    pub bin: Option<PathBuf>,
    /// None = still checking; Some(Err) = it doesn't run.
    pub version: Option<Result<String, String>>,
    /// `hwenc::support_line` (empty when there is nothing to check).
    pub support: String,
}

pub struct View<'a> {
    /// The FFmpeg in use (None = none found).
    pub current: Option<FfmpegChoice>,
    pub mine: Entry,
    pub apps: Entry,
    /// Why switching is off (jobs or an FFmpeg update running).
    pub blocked: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    UseMine,
    UseApps,
    /// Remember and use this folder as "My FFmpeg".
    SetOwn(String),
    /// Get the app's copy (opens the updater).
    Download,
    OpenSettings,
}

/// Text buffer for the My FFmpeg folder box (kept by the app between frames).
#[derive(Default)]
pub struct OwnEdit {
    /// Some while the user is typing.
    pub text: Option<String>,
    /// Problem with the last typed / picked folder.
    pub err: Option<String>,
}

fn version_text(e: &Entry) -> String {
    match &e.version {
        None => "Checking…".into(),
        Some(Ok(v)) => format!("FFmpeg {v}"),
        Some(Err(_)) => "Doesn't run (see the log)".into(),
    }
}

fn detail(ui: &mut egui::Ui, e: &Entry) {
    let mut t = version_text(e);
    if !e.support.is_empty() && matches!(e.version, Some(Ok(_))) {
        t = format!("{t} · {}", e.support);
    }
    let col = if matches!(e.version, Some(Err(_))) { crate::ui::theme::c().err } else { ui.visuals().weak_text_color() };
    ui.add(egui::Label::new(RichText::new(t).color(col)).wrap());
}

fn pick_folder(start: Option<&PathBuf>) -> Option<String> {
    let mut d = rfd::FileDialog::new().set_title("Choose your FFmpeg folder (with ffmpeg.exe and ffprobe.exe)");
    if let Some(s) = start.filter(|s| s.is_dir()) {
        d = d.set_directory(s);
    }
    d.pick_folder().map(|p| p.to_string_lossy().into_owned())
}

/// The Settings section. `own_setting` = the remembered folder as typed.
pub fn settings_section(ui: &mut egui::Ui, v: &View, own_setting: &str, edit: &mut OwnEdit) -> Option<Action> {
    let mut act = None;
    let can = v.blocked.is_none();
    ui.add_enabled_ui(can, |ui| {
        // ---- My FFmpeg
        let mine = v.current == Some(FfmpegChoice::Mine);
        if ui.radio(mine, RichText::new("My FFmpeg").strong()).on_hover_text("Your own FFmpeg folder. The app only reads it; it never updates or changes it.").clicked() && !mine {
            act = Some(if own_setting.trim().is_empty() {
                match pick_folder(None) {
                    Some(d) => Action::SetOwn(d),
                    None => return,
                }
            } else {
                Action::UseMine
            });
        }
        ui.indent("ffc_mine", |ui| {
            ui.horizontal(|ui| {
                let mut buf = edit.text.clone().unwrap_or_else(|| own_setting.to_string());
                let w = (ui.available_width() - 110.0).max(160.0);
                let r = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(w).hint_text("Folder with ffmpeg.exe and ffprobe.exe"));
                let r = r.on_hover_text("Your FFmpeg folder (its bin folder, or the folder above it). Press Enter to use it.");
                if r.has_focus() || r.changed() {
                    edit.text = Some(buf.clone());
                }
                if r.lost_focus() {
                    if buf.trim() != own_setting.trim() && !buf.trim().is_empty() {
                        // the text stays until the folder is accepted (a problem is shown below it)
                        act = Some(Action::SetOwn(buf.trim().to_string()));
                    } else {
                        edit.text = None;
                        edit.err = None;
                    }
                }
                if ui.button("Browse…").clicked() {
                    let start = (!own_setting.trim().is_empty()).then(|| PathBuf::from(own_setting.trim()));
                    if let Some(d) = pick_folder(start.as_ref()) {
                        edit.text = None;
                        act = Some(Action::SetOwn(d));
                    }
                }
            });
            if let Some(e) = &edit.err {
                ui.colored_label(crate::ui::theme::c().err, e);
            }
            if v.mine.bin.is_some() {
                detail(ui, &v.mine);
            } else {
                hint(ui, "Not set. Choose the folder of your own FFmpeg (it needs ffmpeg.exe and ffprobe.exe).");
            }
        });
        ui.add_space(4.0);
        // ---- App's FFmpeg
        let apps = v.current == Some(FfmpegChoice::Apps);
        let installed = v.apps.bin.is_some();
        if ui.radio(apps, RichText::new("App's FFmpeg").strong()).on_hover_text("The copy the app downloads and keeps up to date (see App-managed FFmpeg location below).").clicked() && !apps {
            act = Some(if installed { Action::UseApps } else { Action::Download });
        }
        ui.indent("ffc_apps", |ui| {
            if let Some(b) = &v.apps.bin {
                ui.add(egui::Label::new(RichText::new(b.display().to_string()).monospace()).selectable(true).wrap());
                detail(ui, &v.apps);
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Not downloaded yet.");
                    if ui.button("Download…").on_hover_text("Opens Help › Check for FFmpeg updates…; after installing, the app uses its own copy.").clicked() {
                        act = Some(Action::Download);
                    }
                });
            }
        });
    });
    if let Some(b) = v.blocked {
        hint(ui, b);
    }
    hint(ui, "Switching applies immediately. Your own FFmpeg folder is remembered and never changed, so you can switch back any time, here or in the FFmpeg menu at the top right.");
    act
}

/// Menu-bar drop-down contents.
pub fn menu(ui: &mut egui::Ui, v: &View) -> Option<Action> {
    let mut act = None;
    let can = v.blocked.is_none();
    let mark = |on: bool| if on { "✔ " } else { "    " };
    let mine_on = v.current == Some(FfmpegChoice::Mine);
    let mine_txt = match &v.mine.bin {
        Some(b) => format!("{}My FFmpeg ({})", mark(mine_on), shorten_path(b, 48)),
        None => format!("{}My FFmpeg (not set – choose in Settings)", mark(mine_on)),
    };
    let mine_tip = match &v.mine.bin {
        Some(b) => format!("{}\n{}", b.display(), version_text(&v.mine) + if v.mine.support.is_empty() { "" } else { " · " } + &v.mine.support),
        None => "Choose the folder of your own FFmpeg in Settings › FFmpeg to use.".into(),
    };
    if ui.add_enabled(can && !mine_on, egui::Button::new(mine_txt).selected(mine_on)).on_hover_text(mine_tip).on_disabled_hover_text(v.blocked.unwrap_or("In use.")).clicked() {
        act = Some(if v.mine.bin.is_some() { Action::UseMine } else { Action::OpenSettings });
        ui.close_menu();
    }
    let apps_on = v.current == Some(FfmpegChoice::Apps);
    let apps_txt = match (&v.apps.bin, &v.apps.version) {
        (None, _) => format!("{}App's FFmpeg (not downloaded – Download…)", mark(apps_on)),
        (Some(_), Some(Ok(ver))) => format!("{}App's FFmpeg ({ver})", mark(apps_on)),
        (Some(_), _) => format!("{}App's FFmpeg", mark(apps_on)),
    };
    let apps_tip = match &v.apps.bin {
        Some(b) => format!("{}\n{}", b.display(), version_text(&v.apps) + if v.apps.support.is_empty() { "" } else { " · " } + &v.apps.support),
        None => "Download the app's own FFmpeg (Help › Check for FFmpeg updates…).".into(),
    };
    if ui.add_enabled(can && !apps_on, egui::Button::new(apps_txt).selected(apps_on)).on_hover_text(apps_tip).on_disabled_hover_text(v.blocked.unwrap_or("In use.")).clicked() {
        act = Some(if v.apps.bin.is_some() { Action::UseApps } else { Action::Download });
        ui.close_menu();
    }
    if let Some(b) = v.blocked {
        ui.label(RichText::new(b).small().weak());
    }
    ui.separator();
    if ui.button("FFmpeg settings…").clicked() {
        act = Some(Action::OpenSettings);
        ui.close_menu();
    }
    act
}
