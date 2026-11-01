//! Preset management inside the app: one lazily created form ("editor") per
//! preset, change tracking against the saved values, and the preset dialogs.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};
use ffenc::presets::{self, Preset};
use ffenc::settings::Settings;
use serde_json::Value;

use super::common::hint;
use super::modes::{kinds, new_mode, Ctx, Mode};

pub struct Editor {
    pub mode: Box<dyn Mode>,
    /// Saved preset values as the form serialises them (run fields stripped).
    pub baseline: Value,
}

impl Editor {
    pub fn values(&self) -> Value {
        presets::strip_run_fields(self.mode.save(), self.mode.run_fields())
    }
    pub fn modified(&self) -> bool {
        self.values() != self.baseline
    }
    /// Only the per-run keys of the current form.
    pub fn run_values(&self) -> Value {
        let full = self.mode.save();
        match full {
            Value::Object(m) => Value::Object(m.into_iter().filter(|(k, _)| self.mode.run_fields().contains(&k.as_str())).collect()),
            _ => Value::Null,
        }
    }
}

/// Canonical values of a preset for its form (null → concrete defaults).
pub fn canonical_values(p: &Preset, s: &Settings) -> Option<Value> {
    let mut m = new_mode(&p.kind)?;
    m.load(&p.values, s);
    Some(presets::strip_run_fields(m.save(), m.run_fields()))
}

pub fn make_editor(p: &Preset, s: &Settings, draft: Option<&Value>) -> Option<Editor> {
    let mut mode = new_mode(&p.kind)?;
    mode.load(&p.values, s);
    let baseline = presets::strip_run_fields(mode.save(), mode.run_fields());
    if let Some(d) = draft {
        let merged = presets::overlay(mode.save(), d);
        mode.load(&merged, s);
    }
    Some(Editor { mode, baseline })
}

/// Keys that hold "the" input of a form, used to carry a file across presets.
const PRIMARY_KEYS: &[&str] = &["input", "video", "dest", "image"];

pub fn primary_input(e: &Editor) -> Option<PathBuf> {
    let v = e.mode.save();
    // "input" is always a path; the other keys are paths only in some types
    // (Custom pipeline uses "video" for its video action, e.g. "Copy").
    PRIMARY_KEYS
        .iter()
        .filter_map(|k| v.get(*k).and_then(|x| x.as_str()).map(|s| (*k, s.trim())))
        .find(|(k, s)| !s.is_empty() && (*k == "input" || s.contains(['\\', '/'])))
        .map(|(_, s)| PathBuf::from(s))
}

#[derive(Clone, PartialEq)]
pub enum NewFrom {
    Kind(String),
    Copy(String),
}

/// What happened in the sidebar (the app reacts: save, run, import/export).
#[derive(Debug, Clone, PartialEq)]
pub enum SideEvent {
    Selected,
    /// Order, favorites or folders changed: save the presets file.
    Changed,
    /// Preview the selected preset's job(s).
    Run,
    Import,
    Export { all: bool },
}

pub enum Dialog {
    New { name: String, from: NewFrom },
    SaveAs { name: String },
    Rename { id: String, name: String },
    Delete { id: String },
    Reset { id: String },
    Category { id: String, category: String },
}

pub enum Action {
    None,
    Create { name: String, from: NewFrom },
    SaveAs { name: String },
    Rename { id: String, name: String },
    Delete { id: String },
    Reset { id: String },
    SetCategory { id: String, category: String },
}

pub struct PresetUi {
    pub list: Vec<Preset>,
    pub current: usize,
    pub editors: HashMap<String, Editor>,
    pub dialog: Option<Dialog>,
    /// Sidebar search text.
    pub filter: String,
}

impl PresetUi {
    pub fn cur(&self) -> &Preset {
        &self.list[self.current]
    }
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.list.iter().position(|p| p.id == id)
    }
    pub fn modified(&self, id: &str) -> bool {
        self.editors.get(id).map(|e| e.modified()).unwrap_or(false)
    }

    /// The form of preset `idx` (created on first use).
    pub fn editor(&mut self, idx: usize, s: &Settings) -> &mut Editor {
        let p = self.list[idx].clone();
        if !self.editors.contains_key(&p.id) {
            let e = make_editor(&p, s, s.drafts.get(&p.id))
                .or_else(|| make_editor(&Preset { kind: "custom".into(), ..p.clone() }, s, None))
                .expect("custom kind exists");
            self.editors.insert(p.id.clone(), e);
        }
        self.editors.get_mut(&p.id).unwrap()
    }

    /// Recompute every open form's baseline (after Settings changed); unmodified
    /// forms follow the new defaults, edited ones keep their edits.
    pub fn rebase(&mut self, s: &Settings) {
        let ids: Vec<String> = self.editors.keys().cloned().collect();
        for id in ids {
            let Some(p) = self.list.iter().find(|p| p.id == id).cloned() else { continue };
            let e = self.editors.get(&id).unwrap();
            let draft = if e.modified() { e.mode.save() } else { e.run_values() };
            if let Some(ne) = make_editor(&p, s, Some(&draft)) {
                self.editors.insert(id, ne);
            }
        }
    }

    /// Drafts to persist: edited forms in full, otherwise just the run fields.
    pub fn drafts(&self) -> std::collections::BTreeMap<String, Value> {
        self.editors
            .iter()
            .filter_map(|(id, e)| {
                if e.modified() {
                    return Some((id.clone(), e.mode.save()));
                }
                // unchanged form: remember only the files of the current run
                let Value::Object(m) = e.run_values() else { return None };
                let m: serde_json::Map<String, Value> = m.into_iter().filter(|(_, v)| v.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)).collect();
                (!m.is_empty()).then(|| (id.clone(), Value::Object(m)))
            })
            .collect()
    }

    /// Store the form's values into the preset.
    pub fn save_current(&mut self, s: &Settings) {
        let idx = self.current;
        let e = self.editor(idx, s);
        let v = e.values();
        e.baseline = v.clone();
        self.list[idx].values = v;
    }

    /// Throw away edits (keeps the files of the current run).
    pub fn revert_current(&mut self, s: &Settings) {
        let idx = self.current;
        let e = self.editor(idx, s);
        let merged = presets::overlay(e.mode.save(), &e.baseline.clone());
        e.mode.load(&merged, s);
    }

    /// Switch to another preset, carrying the current input file over when the
    /// target form has none.
    pub fn select(&mut self, idx: usize, cx: &Ctx) {
        if idx == self.current || idx >= self.list.len() {
            return;
        }
        let carry = primary_input(self.editor(self.current, cx.settings));
        self.current = idx;
        let e = self.editor(idx, cx.settings);
        if let Some(p) = carry {
            if primary_input(e).is_none() && p.exists() {
                e.mode.on_drop(&[p], cx);
            }
        }
    }

    /// Group key used for sidebar grouping and moving: favorites first, then folders.
    fn group_of(p: &Preset) -> String {
        if p.pinned {
            "\u{1}fav".into()
        } else {
            p.category.trim().to_string()
        }
    }

    /// Sidebar sections in display order: (heading, preset indices). Favorites
    /// first, then folders in order of first appearance, then the rest. Without
    /// any favorites or folders there is a single section without a heading.
    /// A non-empty `filter` gives one flat list of matches.
    pub fn sections(&self, filter: &str) -> Vec<(Option<String>, Vec<usize>)> {
        let f = filter.trim().to_lowercase();
        if !f.is_empty() {
            let kl = kinds();
            let hits = (0..self.list.len())
                .filter(|&i| {
                    let p = &self.list[i];
                    let kind = kl.iter().find(|k| k.0 == p.kind).map(|k| k.1).unwrap_or("");
                    [p.name.as_str(), kind, p.category.as_str(), p.description.as_str()].iter().any(|s| s.to_lowercase().contains(&f))
                })
                .collect();
            return vec![(None, hits)];
        }
        let fav: Vec<usize> = (0..self.list.len()).filter(|&i| self.list[i].pinned).collect();
        let mut folders: Vec<(String, Vec<usize>)> = vec![];
        let mut rest = vec![];
        for (i, p) in self.list.iter().enumerate().filter(|(_, p)| !p.pinned) {
            let c = p.category.trim();
            if c.is_empty() {
                rest.push(i);
            } else if let Some(g) = folders.iter_mut().find(|g| g.0.eq_ignore_ascii_case(c)) {
                g.1.push(i);
            } else {
                folders.push((c.to_string(), vec![i]));
            }
        }
        if fav.is_empty() && folders.is_empty() {
            return vec![(None, rest)];
        }
        let mut out = vec![];
        if !fav.is_empty() {
            out.push((Some("Favorites".to_string()), fav));
        }
        out.extend(folders.into_iter().map(|(n, v)| (Some(n), v)));
        if !rest.is_empty() {
            out.push((Some("Presets".to_string()), rest));
        }
        out
    }

    /// Move preset `i` one step up (-1) / down (+1) within its sidebar group.
    pub fn move_in_group(&mut self, i: usize, dir: isize) -> bool {
        let g = Self::group_of(&self.list[i]);
        let j = if dir < 0 { (0..i).rev().find(|&j| Self::group_of(&self.list[j]) == g) } else { (i + 1..self.list.len()).find(|&j| Self::group_of(&self.list[j]) == g) };
        let Some(j) = j else { return false };
        let p = self.list.remove(i);
        self.list.insert(j, p);
        // keep the selection on the same preset
        if self.current == i {
            self.current = j;
        } else if i < j && self.current > i && self.current <= j {
            self.current -= 1;
        } else if j < i && self.current >= j && self.current < i {
            self.current += 1;
        }
        true
    }

    fn can_move(&self, i: usize, dir: isize) -> bool {
        let g = Self::group_of(&self.list[i]);
        if dir < 0 {
            (0..i).any(|j| Self::group_of(&self.list[j]) == g)
        } else {
            (i + 1..self.list.len()).any(|j| Self::group_of(&self.list[j]) == g)
        }
    }

    /// The HandBrake-style preset panel. One click selects, double-click (or Run) previews the job.
    pub fn sidebar(&mut self, ui: &mut egui::Ui, cx: &Ctx) -> Option<SideEvent> {
        let mut ev: Option<SideEvent> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Presets").strong());
            ui.label(RichText::new(format!("{}", self.list.len())).small().weak());
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.small_button("New…").on_hover_text("New preset (blank of a type, or a copy)").clicked() {
                self.dialog = Some(Dialog::New { name: String::new(), from: NewFrom::Kind("custom".into()) });
            }
            if ui.small_button("Duplicate").on_hover_text("Copy the selected preset").clicked() {
                self.dialog = Some(Dialog::New { name: String::new(), from: NewFrom::Copy(self.cur().id.clone()) });
            }
            if ui.small_button("Import…").on_hover_text("Import presets from a .json file").clicked() {
                ev = Some(SideEvent::Import);
            }
            ui.menu_button(RichText::new("Export").small(), |ui| {
                if ui.button("Export selected preset…").clicked() {
                    ev = Some(SideEvent::Export { all: false });
                    ui.close_menu();
                }
                if ui.button("Export all presets…").clicked() {
                    ev = Some(SideEvent::Export { all: true });
                    ui.close_menu();
                }
            });
        });
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 24.0).max(60.0);
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search presets…").desired_width(w));
            if ui.add_enabled(!self.filter.is_empty(), egui::Button::new("✖").small()).on_hover_text("Clear search").clicked() {
                self.filter.clear();
            }
        });
        ui.separator();

        let kind_label = |k: &str| kinds().into_iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or("?");
        let sections = self.sections(&self.filter.clone());
        let mut clicked = None;
        let mut run = None;
        let mut move_req: Option<(usize, isize)> = None;
        let mut pin_toggle = None;
        let filtering = !self.filter.trim().is_empty();
        egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).max_height(ui.available_height() - 84.0).show(ui, |ui| {
            // full-width rows with left-aligned names
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            if filtering && sections[0].1.is_empty() {
                hint(ui, "No preset matches.");
            }
            for (heading, idxs) in &sections {
                let mut draw = |ui: &mut egui::Ui| {
                    for &i in idxs {
                        let p = &self.list[i];
                        let modified = self.modified(&p.id);
                        let mut label = String::new();
                        if p.pinned && (filtering || heading.is_none()) {
                            label.push_str("★ ");
                        }
                        label.push_str(&p.name);
                        if modified {
                            label.push_str("  *");
                        }
                        let mut text = RichText::new(label);
                        if modified && i != self.current {
                            text = text.color(Color32::from_rgb(230, 180, 60));
                        }
                        let tip = format!(
                            "Type: {}{}{}\n{}{}\n\nClick: select · Double-click: run · Right-click: more",
                            kind_label(&p.kind),
                            if p.builtin.is_some() { " (built-in)" } else { "" },
                            if p.category.trim().is_empty() { String::new() } else { format!("\nFolder: {}", p.category.trim()) },
                            p.description,
                            if modified { "\n* unsaved changes" } else { "" }
                        );
                        let r = ui.add(egui::SelectableLabel::new(i == self.current, text)).on_hover_text(tip);
                        if r.double_clicked() {
                            run = Some(i);
                        } else if r.clicked() {
                            clicked = Some(i);
                        }
                        let id = p.id.clone();
                        let (builtin, pinned, cat) = (p.builtin.is_some(), p.pinned, p.category.clone());
                        r.context_menu(|ui| {
                            if ui.button("Run (preview command)").clicked() {
                                run = Some(i);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(if pinned { "Remove from favorites" } else { "Pin to favorites" }).clicked() {
                                pin_toggle = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Set folder…").clicked() {
                                self.dialog = Some(Dialog::Category { id: id.clone(), category: cat.clone() });
                                ui.close_menu();
                            }
                            if ui.button("Duplicate").clicked() {
                                self.dialog = Some(Dialog::New { name: String::new(), from: NewFrom::Copy(id.clone()) });
                                ui.close_menu();
                            }
                            if ui.button("Rename…").clicked() {
                                self.dialog = Some(Dialog::Rename { id: id.clone(), name: self.list[i].name.clone() });
                                ui.close_menu();
                            }
                            if ui.add_enabled(self.can_move(i, -1), egui::Button::new("Move up")).clicked() {
                                move_req = Some((i, -1));
                                ui.close_menu();
                            }
                            if ui.add_enabled(self.can_move(i, 1), egui::Button::new("Move down")).clicked() {
                                move_req = Some((i, 1));
                                ui.close_menu();
                            }
                            if ui.button("Export…").clicked() {
                                clicked = Some(i);
                                ev = Some(SideEvent::Export { all: false });
                                ui.close_menu();
                            }
                            if ui.add_enabled(builtin, egui::Button::new("Reset to factory defaults…")).clicked() {
                                self.dialog = Some(Dialog::Reset { id: id.clone() });
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Delete…").clicked() {
                                self.dialog = Some(Dialog::Delete { id: id.clone() });
                                ui.close_menu();
                            }
                        });
                    }
                };
                match heading {
                    Some(h) => {
                        let title = if h == "Favorites" { format!("★ Favorites ({})", idxs.len()) } else { format!("{h} ({})", idxs.len()) };
                        egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("pgroup", h.as_str())).default_open(true).show(ui, |ui| ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| draw(ui)));
                    }
                    None => draw(ui),
                }
            }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            let i = self.current;
            if ui.add_enabled(self.can_move(i, -1), egui::Button::new("⬆").small()).on_hover_text("Move up (within its group)").clicked() {
                move_req = Some((i, -1));
            }
            if ui.add_enabled(self.can_move(i, 1), egui::Button::new("⬇").small()).on_hover_text("Move down (within its group)").clicked() {
                move_req = Some((i, 1));
            }
            let pinned = self.list[i].pinned;
            if ui.small_button(if pinned { "Unpin" } else { "Pin" }).on_hover_text("Favorites are listed at the top").clicked() {
                pin_toggle = Some(i);
            }
        });
        let w = ui.available_width();
        if ui.add_sized([w, 28.0], egui::Button::new(RichText::new("▶  Run selected preset").strong())).on_hover_text("Preview the command for the selected preset with the current input, then add it to the queue (F5)").clicked() {
            run = Some(self.current);
        }
        hint(ui, "Double-click a preset to run it with the current file.");

        if let Some(i) = pin_toggle {
            self.list[i].pinned = !self.list[i].pinned;
            ev = ev.or(Some(SideEvent::Changed));
        }
        if let Some((i, d)) = move_req {
            if self.move_in_group(i, d) {
                ev = ev.or(Some(SideEvent::Changed));
            }
        }
        if let Some(i) = run {
            self.select(i, cx);
            return Some(SideEvent::Run);
        }
        if let Some(i) = clicked {
            self.select(i, cx);
            ev = ev.or(Some(SideEvent::Selected));
        }
        ev
    }

    /// Draw the open dialog; returns what the user confirmed.
    pub fn dialogs(&mut self, ctx: &egui::Context) -> Action {
        let Some(d) = self.dialog.as_mut() else { return Action::None };
        let mut act = Action::None;
        let mut close = false;
        let names: Vec<(String, String)> = self.list.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        let center = egui::Align2::CENTER_CENTER;
        match d {
            Dialog::New { name, from } => {
                egui::Window::new("New preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]).show(ctx, |ui| {
                    egui::Grid::new("np").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        ui.label("Start from");
                        ui.vertical(|ui| {
                            let is_copy = matches!(from, NewFrom::Copy(_));
                            if ui.radio(is_copy, "Copy of an existing preset").clicked() && !is_copy {
                                *from = NewFrom::Copy(names[self.current].0.clone());
                            }
                            if let NewFrom::Copy(id) = from {
                                let sel = names.iter().find(|n| &n.0 == id).map(|n| n.1.clone()).unwrap_or_default();
                                egui::ComboBox::from_id_salt("np_copy").width(260.0).selected_text(sel).show_ui(ui, |ui| {
                                    for (pid, pname) in &names {
                                        ui.selectable_value(id, pid.clone(), pname);
                                    }
                                });
                            }
                            let is_kind = matches!(from, NewFrom::Kind(_));
                            if ui.radio(is_kind, "Blank preset of a type").clicked() && !is_kind {
                                *from = NewFrom::Kind("custom".into());
                            }
                            if let NewFrom::Kind(k) = from {
                                let all = kinds();
                                let sel = all.iter().find(|x| x.0 == k.as_str()).map(|x| x.1).unwrap_or("?");
                                egui::ComboBox::from_id_salt("np_kind").width(260.0).selected_text(sel).show_ui(ui, |ui| {
                                    for (kk, label) in &all {
                                        ui.selectable_value(k, kk.to_string(), *label);
                                    }
                                });
                                hint(ui, "“Custom pipeline” lets you combine video/audio/subtitle actions freely.");
                            }
                        });
                        ui.end_row();
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(name).hint_text("e.g. 1080p archive").desired_width(260.0));
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Create").strong()).clicked() {
                            act = Action::Create { name: name.clone(), from: from.clone() };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::SaveAs { name } => {
                egui::Window::new("Save as new preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label("Saves the current form as a new preset. The original preset keeps its saved values.");
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                    // read before re-focusing: lost_focus() is evaluated live
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !enter && ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Save").strong()).clicked() || enter {
                            act = Action::SaveAs { name: name.clone() };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Rename { id, name } => {
                egui::Window::new("Rename preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]).show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Rename").strong()).clicked() {
                            act = Action::Rename { id: id.clone(), name: name.clone() };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Delete { id } => {
                let n = names.iter().find(|n| &n.0 == id).map(|n| n.1.clone()).unwrap_or_default();
                egui::Window::new("Delete preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Delete the preset “{n}”?"));
                    hint(ui, "Built-ins can be brought back with Presets › Restore missing built-in presets. Export first if you want a copy.");
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Delete").color(Color32::LIGHT_RED)).clicked() {
                            act = Action::Delete { id: id.clone() };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Category { id, category } => {
                let n = names.iter().find(|n| &n.0 == id).map(|n| n.1.clone()).unwrap_or_default();
                let mut folders: Vec<String> = vec![];
                for p in &self.list {
                    let c = p.category.trim();
                    if !c.is_empty() && !folders.iter().any(|f| f.eq_ignore_ascii_case(c)) {
                        folders.push(c.to_string());
                    }
                }
                egui::Window::new("Preset folder").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Folder for “{n}” (leave empty for none):"));
                    let r = ui.add(egui::TextEdit::singleline(category).hint_text("e.g. YouTube, Archive").desired_width(300.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !folders.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Existing:");
                            for f in &folders {
                                if ui.small_button(f).clicked() {
                                    *category = f.clone();
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("OK").strong()).clicked() || enter {
                            act = Action::SetCategory { id: id.clone(), category: category.clone() };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Reset { id } => {
                let n = names.iter().find(|n| &n.0 == id).map(|n| n.1.clone()).unwrap_or_default();
                egui::Window::new("Reset preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Reset “{n}” to its factory name and values?"));
                    hint(ui, "Factory values follow File › Settings defaults (CRF, preset, bitrate, …).");
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Reset").strong()).clicked() {
                            act = Action::Reset { id: id.clone() };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
        }
        if close {
            self.dialog = None;
        }
        act
    }

    /// Apply a confirmed dialog action. Returns an error message to show.
    pub fn apply(&mut self, act: Action, cx: &Ctx) -> Result<(), String> {
        let s = cx.settings;
        match act {
            Action::None => {}
            Action::Create { name, from } => {
                let category = match &from {
                    NewFrom::Copy(id) => self.list.iter().find(|p| &p.id == id).map(|p| p.category.clone()).unwrap_or_default(),
                    NewFrom::Kind(_) => String::new(),
                };
                let (kind, values, desc, base_name) = match &from {
                    NewFrom::Kind(k) => {
                        let label = kinds().into_iter().find(|x| x.0 == k.as_str()).map(|x| x.1).unwrap_or("Preset");
                        let desc = new_mode(k).map(|m| m.description().to_string()).unwrap_or_default();
                        (k.clone(), Value::Null, desc, label.to_string())
                    }
                    NewFrom::Copy(id) => {
                        let p = self.list.iter().find(|p| &p.id == id).cloned().ok_or("Preset not found")?;
                        (p.kind.clone(), canonical_values(&p, s).unwrap_or(Value::Null), p.description.clone(), format!("{} copy", p.name))
                    }
                };
                let name = if name.trim().is_empty() { base_name } else { name.trim().to_string() };
                let p = Preset { id: presets::new_id(), name: presets::unique_name(&self.list, &name), kind, description: desc, builtin: None, values, pinned: false, category };
                let at = self.current + 1;
                self.list.insert(at, p);
                self.select(at, cx);
            }
            Action::SaveAs { name } => {
                let idx = self.current;
                let src = self.list[idx].clone();
                let e = self.editor(idx, s);
                let full = e.mode.save();
                let values = e.values();
                let name = if name.trim().is_empty() { format!("{} copy", src.name) } else { name.trim().to_string() };
                let p = Preset { id: presets::new_id(), name: presets::unique_name(&self.list, &name), kind: src.kind.clone(), description: src.description.clone(), builtin: None, values, pinned: false, category: src.category.clone() };
                // original form goes back to its saved values
                self.revert_current(s);
                let mut ne = make_editor(&p, s, None).ok_or("Unknown preset type")?;
                ne.mode.load(&full, s);
                ne.baseline = ne.values();
                self.editors.insert(p.id.clone(), ne);
                self.list.insert(idx + 1, p.clone());
                self.list[idx + 1].values = self.editors[&p.id].baseline.clone();
                self.current = idx + 1;
            }
            Action::Rename { id, name } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err("The name can't be empty.".into());
                }
                if self.list.iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(&name)) {
                    return Err(format!("There is already a preset called “{name}”."));
                }
                if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
                    p.name = name;
                }
            }
            Action::Delete { id } => {
                if self.list.len() <= 1 {
                    return Err("Keep at least one preset.".into());
                }
                let Some(i) = self.index_of(&id) else { return Ok(()) };
                self.list.remove(i);
                self.editors.remove(&id);
                if self.current >= self.list.len() || self.current > i {
                    self.current = self.current.saturating_sub(1).min(self.list.len() - 1);
                }
            }
            Action::SetCategory { id, category } => {
                if let Some(p) = self.list.iter_mut().find(|p| p.id == id) {
                    p.category = category.trim().to_string();
                }
            }
            Action::Reset { id } => {
                let Some(i) = self.index_of(&id) else { return Ok(()) };
                let Some(f) = self.list[i].builtin.as_deref().and_then(presets::factory) else { return Err("Only built-in presets have factory defaults.".into()) };
                let run = self.editors.get(&id).map(|e| e.run_values());
                let (pinned, category) = (self.list[i].pinned, self.list[i].category.clone());
                self.list[i] = Preset { id: id.clone(), pinned, category, ..f };
                self.editors.remove(&id);
                if let Some(e) = make_editor(&self.list[i], s, run.as_ref()) {
                    self.editors.insert(id, e);
                }
            }
        }
        Ok(())
    }

    /// Values to export for a preset (null resolved to concrete values).
    pub fn export_copy(&self, idx: usize, s: &Settings) -> Preset {
        let p = self.list[idx].clone();
        let values = canonical_values(&p, s).unwrap_or(p.values.clone());
        Preset { values, ..p }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ui_with(list: Vec<Preset>) -> PresetUi {
        PresetUi { list, current: 0, editors: HashMap::new(), dialog: None, filter: String::new() }
    }
    fn set(e: &mut Editor, key: &str, v: Value, s: &Settings) {
        let mut full = e.mode.save();
        full[key] = v;
        e.mode.load(&full, s);
    }

    #[test]
    fn every_kind_roundtrips_stably() {
        let s = Settings::default();
        for p in presets::factory_all() {
            let e = make_editor(&p, &s, None).unwrap_or_else(|| panic!("no form for {}", p.kind));
            assert!(!e.modified(), "{} modified right after load", p.kind);
            let v = canonical_values(&p, &s).unwrap();
            assert!(v.is_object(), "{}", p.kind);
            for f in e.mode.run_fields() {
                assert!(v.get(*f).is_none(), "{}: run field {f} stored in preset", p.kind);
            }
            // loading the concrete values gives the same values again
            let p2 = Preset { values: v.clone(), ..p.clone() };
            assert_eq!(canonical_values(&p2, &s).unwrap(), v, "{}", p.kind);
        }
    }

    #[test]
    fn edit_save_revert_and_run_fields() {
        let s = Settings::default();
        let mut pu = ui_with(presets::factory_all());
        let e = pu.editor(0, &s);
        assert_eq!(e.mode.key(), "reencode");
        set(e, "input", json!("/v/a.mkv"), &s);
        assert!(!pu.modified("builtin-reencode"), "choosing a file is not a preset change");
        let e = pu.editor(0, &s);
        set(e, "trim_start", json!(12.5), &s);
        assert!(pu.modified("builtin-reencode"));
        pu.revert_current(&s);
        assert!(!pu.modified("builtin-reencode"));
        assert_eq!(pu.editor(0, &s).mode.save()["input"], json!("/v/a.mkv"), "revert keeps the file");
        set(pu.editor(0, &s), "trim_start", json!(3.0), &s);
        pu.save_current(&s);
        assert!(!pu.modified("builtin-reencode"));
        assert_eq!(pu.list[0].values["trim_start"], json!(3.0));
        assert!(pu.list[0].values.get("input").is_none());
    }

    #[test]
    fn drafts_and_reload() {
        let s0 = Settings::default();
        let mut pu = ui_with(presets::factory_all());
        set(pu.editor(0, &s0), "input", json!("/v/a.mkv"), &s0);
        let _ = pu.editor(1, &s0); // opened, untouched
        let d = pu.drafts();
        assert_eq!(d.len(), 1);
        assert_eq!(d["builtin-reencode"], json!({"input": "/v/a.mkv"}));
        set(pu.editor(0, &s0), "trim_end", json!(4.0), &s0);
        let d = pu.drafts();
        assert_eq!(d["builtin-reencode"]["trim_end"], json!(4.0));
        // next start: draft restored, still shown as modified
        let s1 = Settings { drafts: d, ..Settings::default() };
        let mut pu2 = ui_with(presets::factory_all());
        let e = pu2.editor(0, &s1);
        assert_eq!(e.mode.save()["input"], json!("/v/a.mkv"));
        assert!(e.modified());
    }

    #[test]
    fn rebase_follows_settings_for_untouched_forms() {
        let s = Settings::default();
        let mut pu = ui_with(presets::factory_all());
        let _ = pu.editor(0, &s);
        set(pu.editor(1, &s), "custom_duration", json!(99.0), &s);
        let s2 = Settings { crf: 30, audio_bitrate_k: 128, ..Settings::default() };
        pu.rebase(&s2);
        assert_eq!(pu.editor(0, &s2).mode.save()["video"]["crf"], json!(30));
        assert!(!pu.modified("builtin-reencode"));
        assert!(pu.modified("builtin-still"), "edits survive");
        assert_eq!(pu.editor(1, &s2).mode.save()["custom_duration"], json!(99.0));
    }

    #[test]
    fn create_saveas_rename_delete_reset() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx };
        let mut pu = ui_with(presets::factory_all());
        let n0 = pu.list.len();
        // blank custom
        pu.apply(Action::Create { name: "".into(), from: NewFrom::Kind("custom".into()) }, &cx).unwrap();
        assert_eq!(pu.list.len(), n0 + 1);
        assert_eq!(pu.current, 1);
        assert_eq!(pu.cur().kind, "custom");
        assert_eq!(pu.cur().name, "Custom pipeline (2)");
        assert!(pu.cur().builtin.is_none());
        // copy of re-encode with edits saved first
        pu.current = 0;
        set(pu.editor(0, &s), "trim_start", json!(7.0), &s);
        pu.save_current(&s);
        pu.apply(Action::Create { name: "My encode".into(), from: NewFrom::Copy("builtin-reencode".into()) }, &cx).unwrap();
        assert_eq!(pu.cur().name, "My encode");
        assert_eq!(pu.cur().values["trim_start"], json!(7.0));
        // save as: edits move to the new preset, original reverts
        pu.current = 0;
        set(pu.editor(0, &s), "input", json!("/v/b.mkv"), &s);
        set(pu.editor(0, &s), "trim_end", json!(9.0), &s);
        pu.apply(Action::SaveAs { name: "Trimmed".into() }, &cx).unwrap();
        assert_eq!(pu.cur().name, "Trimmed");
        assert_eq!(pu.current, 1);
        assert_eq!(pu.cur().values["trim_end"], json!(9.0));
        let id = pu.cur().id.clone();
        assert!(!pu.modified(&id));
        assert_eq!(pu.editor(1, &s).mode.save()["input"], json!("/v/b.mkv"));
        assert!(!pu.modified("builtin-reencode"));
        assert_eq!(pu.editor(0, &s).mode.save()["trim_end"], json!(0.0));
        // rename rules
        assert!(pu.apply(Action::Rename { id: id.clone(), name: "  ".into() }, &cx).is_err());
        assert!(pu.apply(Action::Rename { id: id.clone(), name: "re-encode VIDEO".into() }, &cx).is_err());
        pu.apply(Action::Rename { id: id.clone(), name: "Short".into() }, &cx).unwrap();
        assert_eq!(pu.list[1].name, "Short");
        // reset built-in
        pu.apply(Action::Rename { id: "builtin-reencode".into(), name: "Mine".into() }, &cx).unwrap();
        pu.apply(Action::Reset { id: "builtin-reencode".into() }, &cx).unwrap();
        assert_eq!(pu.list[0].name, "Re-encode video");
        assert!(pu.list[0].values.is_null());
        assert!(pu.apply(Action::Reset { id: id.clone() }, &cx).is_err());
        // delete
        pu.current = 1;
        pu.apply(Action::Delete { id: id.clone() }, &cx).unwrap();
        assert!(pu.index_of(&id).is_none());
        assert!(pu.current < pu.list.len());
        let mut one = ui_with(vec![presets::factory("builtin-mix").unwrap()]);
        assert!(one.apply(Action::Delete { id: "builtin-mix".into() }, &cx).is_err());
    }

    #[test]
    fn select_carries_input_file() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx };
        let f = std::env::temp_dir().join(format!("carry_{}.mkv", ffenc::util::unique_id()));
        std::fs::write(&f, b"x").unwrap();
        let mut pu = ui_with(presets::factory_all());
        set(pu.editor(0, &s), "input", json!(f.to_string_lossy()), &s);
        let mix = pu.index_of("builtin-mix").unwrap();
        pu.select(mix, &cx);
        assert_eq!(pu.editor(mix, &s).mode.save()["input"], json!(f.to_string_lossy()));
        assert!(pu.editor(mix, &s).mode.save()["out_name"].as_str().unwrap().ends_with("_mixed.mkv"));
        // into a Custom-type preset (its "video" key is an action, not a path)
        let add = pu.index_of("builtin-addchapters").unwrap();
        assert!(primary_input(pu.editor(add, &s)).is_none());
        pu.select(add, &cx);
        assert_eq!(pu.editor(add, &s).mode.save()["input"], json!(f.to_string_lossy()));
        assert!(pu.editor(add, &s).mode.save()["out_name"].as_str().unwrap().ends_with("_chapters.mkv"));
        std::fs::remove_file(&f).unwrap();
    }

    #[test]
    fn export_resolves_defaults() {
        let s = Settings { crf: 19, ..Settings::default() };
        let pu = ui_with(presets::factory_all());
        let p = pu.export_copy(0, &s);
        assert_eq!(p.values["video"]["crf"], json!(19));
        assert!(pu.list[0].values.is_null(), "stored preset untouched");
    }

    #[test]
    fn sidebar_sections_filter_and_group_moves() {
        let mut pu = ui_with(presets::factory_all());
        let n = pu.list.len();
        let secs = pu.sections("");
        assert_eq!(secs.len(), 1);
        assert!(secs[0].0.is_none() && secs[0].1 == (0..n).collect::<Vec<_>>(), "user order, no headings");
        // favorites + a folder
        pu.list[3].pinned = true;
        pu.list[5].pinned = true;
        pu.list[1].category = "Audio".into();
        pu.list[4].category = "audio".into();
        let secs = pu.sections("");
        let heads: Vec<_> = secs.iter().map(|s| s.0.clone().unwrap()).collect();
        assert_eq!(heads, ["Favorites", "Audio", "Presets"]);
        assert_eq!(secs[0].1, vec![3, 5]);
        assert_eq!(secs[1].1, vec![1, 4], "folder names match case-insensitively");
        assert!(!secs[2].1.contains(&3) && secs[2].1.contains(&0));
        // search: name, type label, folder, description; flat
        let hit = |q: &str, pu: &PresetUi| pu.sections(q)[0].1.iter().map(|&i| pu.list[i].id.clone()).collect::<Vec<_>>();
        assert_eq!(hit("REMUX", &pu), ["builtin-remux"]);
        assert!(hit("audio", &pu).len() >= 3);
        assert!(hit("custom pipeline", &pu).contains(&"builtin-addchapters".to_string()), "matches the type label");
        assert!(hit("zzz", &pu).is_empty());
        // moving inside the favorites group skips the presets between them
        pu.current = 5;
        assert!(pu.move_in_group(5, -1));
        assert_eq!(pu.sections("")[0].1, vec![3, 4]);
        assert!(pu.list[3].pinned && pu.list[4].pinned);
        assert_eq!(pu.current, 3, "selection follows the moved preset");
        assert!(!pu.move_in_group(3, -1), "already first in its group");
        assert!(!pu.can_move(4, 1));
    }

    #[test]
    fn add_chapters_builtin_is_copy_with_auto_chapters() {
        let s = Settings { output_folder: "/out".into(), ..Settings::default() };
        let p = presets::factory("builtin-addchapters").unwrap();
        let e = make_editor(&p, &s, None).unwrap();
        let v = e.mode.save();
        assert_eq!((v["video"].as_str(), v["audio"].as_str(), v["container"].as_str(), v["chapter_mode"].as_str()), (Some("Copy"), Some("Copy"), Some("same"), Some("Auto")));
        assert_eq!(v["out_dir"], json!("/out"), "partial factory values sit on top of the settings defaults");
        assert_eq!(v["chapter_interval_min"], json!(5.0));
        assert!(!e.modified());
    }

    #[test]
    fn category_action_and_reset_keeps_pin() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx };
        let mut pu = ui_with(presets::factory_all());
        pu.apply(Action::SetCategory { id: "builtin-mix".into(), category: "  Audio ".into() }, &cx).unwrap();
        let i = pu.index_of("builtin-mix").unwrap();
        assert_eq!(pu.list[i].category, "Audio");
        pu.list[i].pinned = true;
        pu.apply(Action::Reset { id: "builtin-mix".into() }, &cx).unwrap();
        assert!(pu.list[i].pinned && pu.list[i].category == "Audio");
        // duplicates inherit the folder, not the pin
        pu.apply(Action::Create { name: "Mix 2".into(), from: NewFrom::Copy("builtin-mix".into()) }, &cx).unwrap();
        assert_eq!(pu.cur().category, "Audio");
        assert!(!pu.cur().pinned);
    }
}
