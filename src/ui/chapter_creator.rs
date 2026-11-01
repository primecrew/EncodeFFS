//! Chapter creator: detects whether the source already has chapters, edits
//! automatic / manual chapters (add at a timestamp, import a text list, load the
//! source's chapters, sort, validate) and previews the exact FFMETADATA1 file
//! that is written to a temp file for ffmpeg (`-f ffmetadata -i … -map_chapters N`).

use eframe::egui::{self, Color32, RichText, Ui};
use ffenc::cmd::chapters::{self, ChapterMode, ManualChapter};
use ffenc::ffmpeg::probe::MediaInfo;
use ffenc::util::fmt_hms;

use super::common::{hint, num_f64};

/// The preset fields the creator edits.
pub struct Spec<'a> {
    pub mode: &'a mut ChapterMode,
    pub interval_min: &'a mut f64,
    pub manual: &'a mut Vec<ManualChapter>,
    pub auto_if_none: &'a mut bool,
}

/// What is known about the input.
#[derive(Default)]
pub struct Source<'a> {
    pub info: Option<&'a MediaInfo>,
    pub probing: bool,
    /// Output duration after trimming, when known.
    pub out_dur: Option<f64>,
    pub trim_start: f64,
    /// Batch: replaces the per-file detection line.
    pub batch_note: Option<String>,
}

#[derive(Default)]
pub struct ChapterCreator {
    open: bool,
    ts: String,
    ts_title: String,
    import_text: String,
    msg: Option<(bool, String)>,
    show_meta: bool,
}

const OK: Color32 = Color32::from_rgb(120, 180, 120);
const BAD: Color32 = Color32::LIGHT_RED;
const WARN: Color32 = Color32::from_rgb(230, 180, 80);

/// One-line detection text + whether the input lacks chapters.
pub fn detection(src: &Source) -> (String, Color32, bool) {
    if let Some(n) = &src.batch_note {
        return (n.clone(), Color32::GRAY, false);
    }
    if src.probing {
        return ("checking with ffprobe…".into(), Color32::GRAY, false);
    }
    match src.info {
        None => ("choose an input to check for chapters".into(), Color32::GRAY, false),
        Some(i) if i.chapters.is_empty() => ("No chapters found in this file".into(), WARN, true),
        Some(i) => (format!("{} chapter(s) found in this file", i.chapters.len()), OK, false),
    }
}

/// Validation line for the current settings (None when nothing to say).
pub fn status(spec: &Spec, src: &Source) -> Option<(String, Color32)> {
    let has = src.info.map(|i| i.has_chapters()).unwrap_or(true);
    let mode = if *spec.mode == ChapterMode::Off && *spec.auto_if_none && !has { ChapterMode::Auto } else { *spec.mode };
    if mode == ChapterMode::Off {
        return None;
    }
    let d = src.out_dur?;
    if d <= 0.0 {
        return Some(("Trim is longer than the input".into(), BAD));
    }
    match chapters::build_chapters(d, mode, *spec.interval_min * 60.0, spec.manual).and_then(|l| chapters::validate(&l, d).map(|_| l)) {
        Ok(l) if l.is_empty() => Some(("No chapters would be generated".into(), WARN)),
        Ok(l) => Some((format!("OK: {} chapter(s), last one ends at {}", l.len(), fmt_hms(d)), OK)),
        Err(e) => Some((e, BAD)),
    }
}

impl ChapterCreator {
    /// Rows for a two-column `Grid`, then (outside the grid) the creator window.
    pub fn grid_rows(&mut self, ui: &mut Ui, id: &str, spec: &mut Spec, src: &Source) {
        let (det, col, none) = detection(src);
        ui.label("Source chapters");
        ui.horizontal(|ui| {
            ui.label(RichText::new(det).color(col));
            if none && *spec.mode == ChapterMode::Off && ui.button("Create chapters…").clicked() {
                if spec.manual.is_empty() {
                    *spec.mode = ChapterMode::Auto;
                }
                self.open = true;
            }
        });
        ui.end_row();

        ui.label("Chapters");
        ui.horizontal(|ui| {
            mode_combo(ui, &format!("{id}_mode"), spec.mode);
            if ui.button("Chapter creator…").on_hover_text("Auto / manual chapters, import a list, preview and validate").clicked() {
                self.open = true;
            }
        });
        ui.end_row();
        if spec.mode.auto() {
            ui.label("Interval");
            num_f64(ui, spec.interval_min, 0.1..=600.0, 0.1, " min");
            ui.end_row();
        }
        if *spec.mode == ChapterMode::Off {
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(spec.auto_if_none, "If the input has no chapters, create one every");
                ui.add_enabled_ui(*spec.auto_if_none, |ui| num_f64(ui, spec.interval_min, 0.1..=600.0, 0.1, " min"));
            });
            ui.end_row();
        }
        if spec.mode.manual() {
            ui.label("Manual chapters");
            ui.horizontal(|ui| {
                let bad = spec.manual.iter().filter(|m| chapters::parse_hms(&m.time).is_err()).count();
                ui.label(format!("{} entr{}", spec.manual.len(), if spec.manual.len() == 1 { "y" } else { "ies" }));
                if bad > 0 {
                    ui.colored_label(BAD, format!("({bad} invalid time{})", if bad == 1 { "" } else { "s" }));
                }
                if ui.small_button("Edit…").clicked() {
                    self.open = true;
                }
            });
            ui.end_row();
        }
        if let Some((t, c)) = status(spec, src) {
            ui.label("");
            ui.label(RichText::new(t).color(c).small());
            ui.end_row();
        }
        self.window(ui.ctx(), id, spec, src);
    }

    /// The floating "Chapter creator" window.
    pub fn window(&mut self, ctx: &egui::Context, id: &str, spec: &mut Spec, src: &Source) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Chapter creator")
            .id(egui::Id::new(("chapter_creator", id)))
            .open(&mut open)
            .default_size([680.0, 600.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.body(ui, id, spec, src));
            });
        self.open = open;
    }

    fn body(&mut self, ui: &mut Ui, id: &str, spec: &mut Spec, src: &Source) {
        let (det, col, _) = detection(src);
        ui.horizontal(|ui| {
            ui.label("Source:");
            ui.label(RichText::new(det).color(col));
        });
        match src.out_dur {
            Some(d) => hint(ui, format!("Output duration after trim: {} – chapter times are relative to the trimmed output.", fmt_hms(d))),
            None => hint(ui, "Output duration unknown (choose an input); times are relative to the trimmed output."),
        }
        if src.info.map(|i| i.has_chapters()).unwrap_or(false) {
            hint(ui, "Generated chapters replace the file's own chapters in the output.");
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Mode");
            for m in ChapterMode::ALL {
                ui.radio_value(spec.mode, m, m.label());
            }
        });
        if spec.mode.auto() {
            ui.horizontal(|ui| {
                ui.label("Every");
                num_f64(ui, spec.interval_min, 0.1..=600.0, 0.1, " min");
                hint(ui, "untitled chapters are named \"Chapter N\" by position");
            });
        }
        if *spec.mode == ChapterMode::Off {
            ui.horizontal(|ui| {
                ui.checkbox(spec.auto_if_none, "Auto-create when the input has none, every");
                num_f64(ui, spec.interval_min, 0.1..=600.0, 0.1, " min");
            });
            hint(ui, "Switch the mode to Manual or Auto + manual to use the list below.");
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Manual chapters").strong());
        egui::Grid::new(format!("{id}_cc_grid")).num_columns(5).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("#");
            ui.label("Time (hh:mm:ss[.mmm])");
            ui.label("Title");
            ui.label("");
            ui.label("");
            ui.end_row();
            let mut remove = None;
            let mut swap = None;
            let n = spec.manual.len();
            for (i, c) in spec.manual.iter_mut().enumerate() {
                ui.label(format!("{}", i + 1));
                let bad = !c.time.trim().is_empty() && chapters::parse_hms(&c.time).is_err();
                let mut te = egui::TextEdit::singleline(&mut c.time).hint_text("00:15:30");
                if bad {
                    te = te.text_color(BAD);
                }
                // fixed sizes: a Grid cell is otherwise limited to last frame's column width
                ui.add_sized([120.0, 20.0], te).on_hover_text("hh:mm:ss or hh:mm:ss.mmm");
                ui.add_sized([300.0, 20.0], egui::TextEdit::singleline(&mut c.title).hint_text("Chapter N"));
                ui.horizontal(|ui| {
                    if ui.add_enabled(i > 0, egui::Button::new("⬆").small()).clicked() {
                        swap = Some(i - 1);
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new("⬇").small()).clicked() {
                        swap = Some(i);
                    }
                    if ui.small_button("✖").on_hover_text("Delete").clicked() {
                        remove = Some(i);
                    }
                });
                if bad {
                    ui.colored_label(BAD, "invalid time");
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
            if let Some(i) = remove {
                spec.manual.remove(i);
            }
            if let Some(i) = swap {
                spec.manual.swap(i, i + 1);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("+ Add row").clicked() {
                spec.manual.push(ManualChapter::default());
                self.manual_on(spec);
            }
            if ui.button("Sort by time").clicked() {
                chapters::sort_manual(spec.manual);
            }
            if ui.add_enabled(!spec.manual.is_empty(), egui::Button::new("Clear")).clicked() {
                spec.manual.clear();
            }
            if let Some(i) = src.info.filter(|i| i.has_chapters()) {
                if ui.button(format!("Load the file's {} chapter(s)", i.chapters.len())).on_hover_text("Copy the existing chapters into the list (shifted by the trim start)").clicked() {
                    let d = src.out_dur.unwrap_or(f64::MAX);
                    let list: Vec<(f64, String)> = i.chapters.iter().map(|c| (c.start, c.title.clone())).collect();
                    *spec.manual = chapters::from_source(&list, src.trim_start, d);
                    self.manual_on(spec);
                    self.msg = Some((true, format!("Loaded {} chapter(s) from the file.", spec.manual.len())));
                }
            }
        });

        ui.add_space(6.0);
        ui.label(RichText::new("Set from timestamp").strong());
        ui.horizontal(|ui| {
            let r1 = ui.add(egui::TextEdit::singleline(&mut self.ts).hint_text("750.5 / 12:30 / 1:02:03.250 / 1h2m3s").desired_width(200.0));
            let r2 = ui.add(egui::TextEdit::singleline(&mut self.ts_title).hint_text("Title (optional)").desired_width(200.0));
            let enter = (r1.lost_focus() || r2.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Add chapter").clicked() || (enter && !self.ts.trim().is_empty()) {
                match chapters::parse_timestamp(&self.ts) {
                    Ok(ms) => {
                        spec.manual.push(ManualChapter { time: chapters::fmt_ms(ms), title: self.ts_title.trim().to_string() });
                        chapters::sort_manual(spec.manual);
                        self.manual_on(spec);
                        self.msg = Some((true, format!("Added {}", chapters::fmt_ms(ms))));
                        self.ts.clear();
                        self.ts_title.clear();
                    }
                    Err(e) => self.msg = Some((false, e)),
                }
            }
        });
        hint(ui, "Paste a time from your player (seconds, mm:ss, hh:mm:ss.mmm or 1h2m3s); it is normalised to hh:mm:ss.mmm.");

        ui.add_space(6.0);
        egui::CollapsingHeader::new("Import from text").id_salt(format!("{id}_cc_imp")).show(ui, |ui| {
            hint(ui, "One chapter per line: \"00:12:30 Title\" (also \"00:12:30 - Title\" or YouTube-style \"0:00 Intro\"). Lines starting with # are ignored.");
            ui.add(egui::TextEdit::multiline(&mut self.import_text).desired_rows(5).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            ui.horizontal(|ui| {
                let mut go = None;
                if ui.button("Append").clicked() {
                    go = Some(false);
                }
                if ui.button("Replace list").clicked() {
                    go = Some(true);
                }
                if ui.button("Open .txt…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Text", &["txt", "ffmeta"]).add_filter("All files", &["*"]).pick_file() {
                        match std::fs::read_to_string(&p) {
                            Ok(t) if t.starts_with(";FFMETADATA1") => match chapters::parse_ffmetadata(&t) {
                                Ok(l) => {
                                    self.import_text = l.iter().map(|c| format!("{} {}", chapters::fmt_ms(c.start_ms), c.title.replace('\n', " "))).collect::<Vec<_>>().join("\n");
                                }
                                Err(e) => self.msg = Some((false, e)),
                            },
                            Ok(t) => self.import_text = t,
                            Err(e) => self.msg = Some((false, format!("{}: {e}", p.display()))),
                        }
                    }
                }
                if ui.add_enabled(!spec.manual.is_empty(), egui::Button::new("Copy list as text")).clicked() {
                    let t = spec.manual.iter().map(|m| format!("{} {}", m.time.trim(), m.title)).collect::<Vec<_>>().join("\n");
                    ui.ctx().copy_text(t);
                    self.msg = Some((true, "Copied.".into()));
                }
                if let Some(replace) = go {
                    match chapters::parse_chapter_list(&self.import_text) {
                        Ok(l) if l.is_empty() => self.msg = Some((false, "Nothing to import.".into())),
                        Ok(l) => {
                            let n = l.len();
                            if replace {
                                *spec.manual = l;
                            } else {
                                spec.manual.extend(l);
                                chapters::sort_manual(spec.manual);
                            }
                            self.manual_on(spec);
                            self.msg = Some((true, format!("Imported {n} chapter(s).")));
                        }
                        Err(e) => self.msg = Some((false, e)),
                    }
                }
            });
        });

        if let Some((ok, m)) = &self.msg {
            ui.colored_label(if *ok { OK } else { BAD }, m);
        }

        ui.separator();
        ui.label(RichText::new("Preview").strong());
        let has = src.info.map(|i| i.has_chapters()).unwrap_or(true);
        let mode = if *spec.mode == ChapterMode::Off && *spec.auto_if_none && !has { ChapterMode::Auto } else { *spec.mode };
        if mode == ChapterMode::Off {
            hint(ui, "Chapters are off: the output keeps the input's chapters (if any).");
            return;
        }
        let Some(d) = src.out_dur.filter(|d| *d > 0.0) else {
            hint(ui, "Choose an input to preview the final chapter list.");
            return;
        };
        match chapters::build_chapters(d, mode, *spec.interval_min * 60.0, spec.manual).and_then(|l| chapters::validate(&l, d).map(|_| l)) {
            Err(e) => {
                ui.colored_label(BAD, e);
            }
            Ok(list) => {
                ui.colored_label(OK, format!("{} chapter(s) – valid: increasing, no zero-length, last ends at {}", list.len(), fmt_hms(d)));
                egui::Grid::new(format!("{id}_cc_prev")).num_columns(5).striped(true).spacing([12.0, 2.0]).show(ui, |ui| {
                    for h in ["#", "Start", "End", "Length", "Title"] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for (i, c) in list.iter().enumerate() {
                        ui.label(format!("{}", i + 1));
                        ui.monospace(chapters::fmt_ms(c.start_ms));
                        ui.monospace(chapters::fmt_ms(c.end_ms));
                        ui.monospace(chapters::fmt_ms(c.end_ms - c.start_ms));
                        ui.label(&c.title);
                        ui.end_row();
                    }
                });
                ui.checkbox(&mut self.show_meta, "Show the FFMETADATA1 file");
                if self.show_meta {
                    let mut t = chapters::ffmetadata(&list);
                    ui.add(egui::TextEdit::multiline(&mut t).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false));
                    hint(ui, "Written as UTF-8 (no BOM) to a temp .ffmeta file for the job and deleted afterwards.");
                }
            }
        }
    }

    /// Editing the manual list implies a manual mode.
    fn manual_on(&self, spec: &mut Spec) {
        *spec.mode = match *spec.mode {
            ChapterMode::Off | ChapterMode::Manual => ChapterMode::Manual,
            _ => ChapterMode::Both,
        };
    }
}

pub fn mode_combo(ui: &mut Ui, id: &str, mode: &mut ChapterMode) {
    egui::ComboBox::from_id_salt(id).selected_text(mode.label()).show_ui(ui, |ui| {
        for m in ChapterMode::ALL {
            ui.selectable_value(mode, m, m.label());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffenc::ffmpeg::probe::ProbeChapter;

    #[test]
    fn detection_and_status() {
        let mut mode = ChapterMode::Off;
        let mut iv = 1.0;
        let mut manual = vec![];
        let mut aif = true;
        let none = MediaInfo::default();
        let src = Source { info: Some(&none), out_dur: Some(150.0), ..Default::default() };
        assert!(detection(&src).2);
        let spec = Spec { mode: &mut mode, interval_min: &mut iv, manual: &mut manual, auto_if_none: &mut aif };
        let (t, _) = status(&spec, &src).unwrap();
        assert!(t.starts_with("OK: 3 chapter"), "{t}");
        let with = MediaInfo { chapters: vec![ProbeChapter { start: 0.0, end: 1.0, title: "x".into() }], ..Default::default() };
        let src2 = Source { info: Some(&with), out_dur: Some(150.0), ..Default::default() };
        assert!(!detection(&src2).2);
        assert!(status(&spec, &src2).is_none(), "has chapters -> auto-if-none does nothing");
        *spec.mode = ChapterMode::Manual;
        spec.manual.push(ManualChapter { time: "00:05:00".into(), title: String::new() });
        assert_eq!(status(&spec, &src).unwrap().1, BAD, "manual chapter beyond the output");
    }
}
