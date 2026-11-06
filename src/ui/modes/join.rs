//! Join / stitch several files into one (concat demuxer or concat filter).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};

use super::{default_audio, default_video, restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::common::{AudioEncode, VideoEncode};
use ffenc::cmd::join::{self, JoinInput, JoinMethod, JoinParams};
use ffenc::ffmpeg::probe;
use ffenc::ffmpeg::runner::JobSpec;
use ffenc::settings::Settings;
use ffenc::util::{fmt_hms, has_ext, stem, ANY_VIDEO_EXTS};

const CONTAINERS: &[&str] = &["mp4", "mkv", "mov"];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct State {
    files: Vec<String>,
    out_dir: String,
    out_name: String,
    last_auto: String,
    suffix: String,
    container: String,
    method: JoinMethod,
    chapters_per_file: bool,
    faststart: bool,
    video: VideoEncode,
    audio: AudioEncode,
}

impl Default for State {
    fn default() -> Self {
        State {
            files: vec![],
            out_dir: String::new(),
            out_name: String::new(),
            last_auto: String::new(),
            suffix: "_joined".into(),
            container: "mp4".into(),
            method: JoinMethod::Copy,
            chapters_per_file: true,
            faststart: true,
            video: VideoEncode::default(),
            audio: AudioEncode::default(),
        }
    }
}

enum Probe {
    Busy(Bg<Result<JoinInput, String>>),
    Done(Result<JoinInput, String>),
}

#[derive(Default)]
pub struct JoinMode {
    st: State,
    probes: HashMap<PathBuf, Probe>,
}

/// Move `v[from]` so it ends up just before the item currently at `before`
/// (`before == v.len()` = to the end).
pub fn move_item<T>(v: &mut Vec<T>, from: usize, before: usize) {
    if from >= v.len() || before > v.len() || before == from || before == from + 1 {
        return;
    }
    let item = v.remove(from);
    let to = if before > from { before - 1 } else { before };
    v.insert(to, item);
}

fn modified(p: &str) -> std::time::SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

/// Six-dot grip; returns the drag response.
fn drag_handle(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::drag());
    let c = if resp.hovered() || resp.dragged() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    for i in 0..3 {
        for j in 0..2 {
            ui.painter().circle_filled(egui::pos2(rect.left() + 3.5 + j as f32 * 5.0, rect.center().y - 5.0 + i as f32 * 5.0), 1.4, c);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    named(resp, egui::WidgetType::DragValue, "Drag handle: drag to reorder (or use the Move up / Move down buttons)")
}

impl JoinMode {
    fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            let sp = s(&p);
            if p.is_file() && has_ext(&p, ANY_VIDEO_EXTS) && !self.st.files.contains(&sp) {
                self.st.files.push(sp);
            }
        }
        self.update_name();
    }
    fn update_name(&mut self) {
        if let Some(f) = self.st.files.first() {
            let n = format!("{}{}.{}", stem(Path::new(f)), self.st.suffix, self.st.container);
            auto_name(&mut self.st.out_name, &mut self.st.last_auto, n);
        }
    }
    fn poll(&mut self, cx: &Ctx) {
        for f in &self.st.files {
            let p = PathBuf::from(f);
            match self.probes.get(&p) {
                None => {
                    let pr = match cx.ffprobe {
                        Some(fp) => {
                            let (fp, q) = (fp.to_path_buf(), p.clone());
                            Probe::Busy(Bg::spawn(cx.egui, move || probe::probe(&fp, &q).and_then(|i| JoinInput::from_info(&q, &i))))
                        }
                        None => Probe::Done(Err("FFprobe not found – set the FFmpeg folder in Settings.".into())),
                    };
                    self.probes.insert(p, pr);
                }
                Some(Probe::Busy(b)) => {
                    if let Some(r) = b.poll() {
                        self.probes.insert(p, Probe::Done(r));
                    }
                }
                Some(Probe::Done(_)) => {}
            }
        }
    }
    fn input(&self, f: &str) -> Option<&Result<JoinInput, String>> {
        match self.probes.get(Path::new(f)) {
            Some(Probe::Done(r)) => Some(r),
            _ => None,
        }
    }
    fn inputs(&self) -> Result<Vec<JoinInput>, String> {
        self.st
            .files
            .iter()
            .map(|f| match self.input(f) {
                Some(Ok(i)) => Ok(i.clone()),
                Some(Err(e)) => Err(format!("{f}: {e}")),
                None => Err(format!("{}: still probing, try again in a moment", Path::new(f).file_name().unwrap_or_default().to_string_lossy())),
            })
            .collect()
    }
}

impl Mode for JoinMode {
    fn key(&self) -> &'static str {
        "join"
    }
    fn title(&self) -> &'static str {
        "Join / stitch files"
    }
    fn description(&self) -> &'static str {
        "Concatenate several files into one: lossless (concat demuxer) or re-encode (concat filter). Optional chapter per file."
    }
    fn needs_encoders(&self) -> bool {
        self.st.method == JoinMethod::Reencode
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["files", "out_name", "last_auto"]
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        self.poll(cx);
        if self.st.out_name.trim().is_empty() && !self.st.files.is_empty() {
            self.update_name();
        }
        let hovering = files_hovering(ui.ctx());
        section(ui, "Files (joined top to bottom)");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add files…").clicked() {
                let mut d = rfd::FileDialog::new().add_filter("Video", ANY_VIDEO_EXTS).add_filter("All files", &["*"]);
                if !cx.settings.input_folder.is_empty() {
                    d = d.set_directory(&cx.settings.input_folder);
                }
                if let Some(mut v) = d.pick_files() {
                    ffenc::util::natural_sort_paths(&mut v);
                    self.add(v);
                }
            }
            if ui.button("Add folder…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.add(ffenc::util::expand_dropped(&[d]));
                }
            }
            ui.separator();
            let some = !self.st.files.is_empty();
            if ui.add_enabled(some, egui::Button::new("Sort by name")).on_hover_text("Natural order: file2 before file10").clicked() {
                self.st.files.sort_by(|a, b| ffenc::util::natural_cmp(&stem(Path::new(a)), &stem(Path::new(b))).then_with(|| ffenc::util::natural_cmp(a, b)));
                self.update_name();
            }
            if ui.add_enabled(some, egui::Button::new("Sort by modified date")).on_hover_text("Oldest first").clicked() {
                self.st.files.sort_by_key(|f| modified(f));
                self.update_name();
            }
            if ui.add_enabled(some, egui::Button::new("Reverse")).clicked() {
                self.st.files.reverse();
                self.update_name();
            }
            if ui.add_enabled(some, egui::Button::new("Clear")).clicked() {
                self.st.files.clear();
            }
        });

        // the list (also the drop zone)
        let frame = egui::Frame::group(ui.style()).inner_margin(6.0);
        let out = frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if self.st.files.is_empty() {
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Drop video files or a folder here, or use Add files…").size(15.0));
                    hint(ui, "Files are added in natural name order (file2 before file10). Drag the dotted grip or use ⬆ ⬇ to reorder.");
                });
                ui.add_space(18.0);
                return;
            }
            let mut action: Option<(usize, usize)> = None; // (from, before)
            let mut remove: Option<usize> = None;
            let n = self.st.files.len();
            egui::ScrollArea::vertical().max_height(300.0).id_salt("joinfiles").show(ui, |ui| {
                for i in 0..n {
                    let f = self.st.files[i].clone();
                    let row = ui.horizontal(|ui| {
                        let h = drag_handle(ui);
                        h.dnd_set_drag_payload(i);
                        ui.label(RichText::new(format!("{:>2}.", i + 1)).monospace());
                        if icon_button(ui, i > 0, "⬆", &format!("Move file {} up", i + 1)).clicked() {
                            action = Some((i, i - 1));
                        }
                        if icon_button(ui, i + 1 < n, "⬇", &format!("Move file {} down", i + 1)).clicked() {
                            action = Some((i, i + 2));
                        }
                        if icon_button(ui, true, "✖", &format!("Remove file {} from the list", i + 1)).clicked() {
                            remove = Some(i);
                        }
                        match self.input(&f) {
                            Some(Ok(j)) => {
                                ui.label(RichText::new(fmt_hms(j.duration_s)).monospace().color(crate::ui::theme::c().ok));
                            }
                            Some(Err(e)) => {
                                ui.label(RichText::new("error").color(crate::ui::theme::c().err)).on_hover_text(e);
                            }
                            None => {
                                ui.spinner();
                            }
                        }
                        ui.label(Path::new(&f).file_name().unwrap_or_default().to_string_lossy()).on_hover_text(&f);
                        if let Some(Ok(j)) = self.input(&f) {
                            let mut d = vec![];
                            if let Some(v) = &j.video {
                                d.push(format!("{} {}x{}{}", v.codec, v.width, v.height, v.fps.map(|f| format!(" {}fps", ffenc::util::fmt_num((f * 100.0).round() / 100.0))).unwrap_or_default()));
                            }
                            if let Some(a) = &j.audio {
                                d.push(format!("{} {}Hz {}ch", a.codec, a.sample_rate.unwrap_or(0), a.channels.unwrap_or(0)));
                            } else {
                                d.push("no audio".into());
                            }
                            hint(ui, d.join(" · "));
                        }
                    });
                    let r = row.response;
                    if let (Some(pos), Some(from)) = (ui.ctx().pointer_interact_pos(), r.dnd_hover_payload::<usize>()) {
                        let before = if pos.y < r.rect.center().y { i } else { i + 1 };
                        let y = if before == i { r.rect.top() } else { r.rect.bottom() };
                        ui.painter().hline(r.rect.x_range(), y, egui::Stroke::new(2.0_f32, crate::ui::theme::c().accent));
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        if r.dnd_release_payload::<usize>().is_some() {
                            action = Some((*from, before));
                        }
                    }
                }
            });
            if let Some((from, before)) = action {
                move_item(&mut self.st.files, from, before);
                self.update_name();
            }
            if let Some(i) = remove {
                self.st.files.remove(i);
                self.update_name();
            }
        });
        if hovering {
            paint_drop_target(ui, out.response.rect);
        }

        // totals + compatibility
        let ready: Vec<JoinInput> = self.st.files.iter().filter_map(|f| self.input(f).and_then(|r| r.as_ref().ok()).cloned()).collect();
        if !self.st.files.is_empty() {
            let pending = self.st.files.len() - ready.len();
            ui.label(format!(
                "{} file(s), total duration {}{}",
                self.st.files.len(),
                fmt_hms(join::total_duration(&ready)),
                if pending > 0 { format!(" ({pending} still probing or failed)") } else { String::new() }
            ));
        }
        let issues = join::compat_issues(&ready);
        if !issues.is_empty() {
            ui.horizontal_wrapped(|ui| {
                warn(ui, format!("The files are not identical in format ({} difference(s)) – a lossless join may fail or play badly.", issues.len()));
                if self.st.method == JoinMethod::Copy && ui.button("Switch to re-encode join").clicked() {
                    self.st.method = JoinMethod::Reencode;
                }
            });
            crate::ui::layout::section(ui, "join.differences", egui::CollapsingHeader::new("Show differences"), false, |ui| {
                for i in &issues {
                    hint(ui, i);
                }
            });
        } else if ready.len() >= 2 && ready.len() == self.st.files.len() {
            ui.label(RichText::new("✔ All files share codecs, resolution, frame rate and audio format – lossless join is safe.").small().color(crate::ui::theme::c().ok));
        }

        section(ui, "Method");
        for m in [JoinMethod::Copy, JoinMethod::Reencode] {
            ui.radio_value(&mut self.st.method, m, m.label());
        }
        if self.st.method == JoinMethod::Reencode {
            hint(ui, "Every file is scaled/padded to the first file's size and frame rate; audio is resampled to its rate (mono or stereo).");
            section(ui, &format!("Video ({})", self.st.video.codec.encoder()));
            egui::Grid::new("joinv").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| crate::ui::codecs_ui::video_codec_ui(ui, "join", &mut self.st.video, cx.encoders.as_ref(), None));
            section(ui, "Audio");
            egui::Grid::new("joina").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| crate::ui::codecs_ui::audio_codec_ui(ui, "joina", &mut self.st.audio, cx.encoders.as_ref(), false));
        }

        section(ui, "Chapters & output");
        let mut rename = false;
        egui::Grid::new("joino").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            crate::ui::common::row_label(ui, "Chapters");
            ui.checkbox(&mut self.st.chapters_per_file, "One chapter per source file (titled with the file name)");
            ui.end_row();
            crate::ui::common::row_label(ui, "Container");
            ui.horizontal(|ui| {
                for c in CONTAINERS {
                    rename |= ui.radio_value(&mut self.st.container, c.to_string(), *c).changed();
                }
            });
            ui.end_row();
            rename |= output_rows_sfx(ui, &mut self.st.out_dir, &mut self.st.out_name, &mut self.st.suffix, &cx.settings.output_folder);
            let mp4 = self.st.container != "mkv";
            faststart_ui(ui, &mut self.st.faststart, mp4);
        });
        if rename {
            // keep a typed name, but follow the container
            let n = self.st.out_name.trim().to_string();
            if !n.is_empty() && n != self.st.last_auto {
                self.st.out_name = format!("{}.{}", stem(Path::new(&n)), self.st.container);
            }
            self.update_name();
        }
    }
    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        if self.st.method == JoinMethod::Reencode {
            crate::ui::codecs_ui::check_available(cx.encoders.as_ref(), Some(&self.st.video), Some(&self.st.audio))?;
        }
        if self.st.files.len() < 2 {
            return Err("Add at least two files to join.".into());
        }
        for f in &self.st.files {
            if !Path::new(f).is_file() {
                return Err(format!("File not found: {f}"));
            }
        }
        let inputs = self.inputs()?;
        let mut name = self.st.out_name.trim().to_string();
        if !CONTAINERS.contains(&ffenc::util::ext_lower(Path::new(&name)).as_str()) {
            name = format!("{}.{}", stem(Path::new(&name)), self.st.container);
        }
        let dir = if self.st.out_dir.trim().is_empty() { Path::new(&self.st.files[0]).parent().map(s).unwrap_or_default() } else { self.st.out_dir.clone() };
        let output = check_output(&dir, &name)?;
        let pr = JoinParams {
            inputs,
            output: output.clone(),
            method: self.st.method,
            chapters_per_file: self.st.chapters_per_file,
            video: self.st.video.clone(),
            audio: self.st.audio.clone(),
            faststart: self.st.faststart,
            list_file: join::temp_list_path(),
            chapter_file: ffenc::cmd::chapters::temp_chapter_path(),
        };
        let cmd = join::build(&pr)?;
        Ok(Plan {
            jobs: vec![JobSpec::new(format!("Join {} files -> {name}", pr.inputs.len()), cmd, output.clone(), log_path_for(&output), Some(pr.output_duration()))],
            ..Default::default()
        })
    }
    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        // folders were already expanded (natural order); several loose files come in
        // Explorer's order, which is not reliable – sort those naturally too
        let mut v: Vec<PathBuf> = paths.iter().filter(|p| has_ext(p, ANY_VIDEO_EXTS)).cloned().collect();
        ffenc::util::natural_sort_paths(&mut v);
        self.add(v);
    }
    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State { out_dir: s.output_folder.clone(), video: default_video(s), audio: default_audio(s), ..Default::default() });
        self.probes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder() {
        let base = vec!['a', 'b', 'c', 'd'];
        let mv = |from, before| {
            let mut v = base.clone();
            move_item(&mut v, from, before);
            v.into_iter().collect::<String>()
        };
        assert_eq!(mv(0, 2), "bacd"); // drag a below b
        assert_eq!(mv(0, 4), "bcda"); // to the end
        assert_eq!(mv(3, 0), "dabc"); // to the top
        assert_eq!(mv(2, 1), "acbd"); // move up
        assert_eq!(mv(1, 3), "acbd"); // move down (before = i + 2)
        assert_eq!(mv(1, 1), "abcd");
        assert_eq!(mv(1, 2), "abcd"); // dropping onto its own lower edge
        assert_eq!(mv(9, 0), "abcd");
    }
}
