//! Preset management inside the app: one lazily created form ("editor") per
//! preset, change tracking against the saved values, and the preset dialogs.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, RichText};
use ffenc::presets::{self, Preset};
use ffenc::settings::Settings;
use serde_json::Value;

use super::common::{hint, icon_button, named};
use super::modes::{kinds, new_mode, Ctx, Mode};

pub struct Editor {
    pub mode: Box<dyn Mode>,
    /// Saved preset values as the form serialises them (run fields stripped).
    pub baseline: Value,
}

impl Editor {
    pub fn values(&self) -> Value {
        presets::strip_folder_fields(presets::strip_run_fields(self.mode.save(), self.mode.run_fields()))
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
    apply_app_folders(&mut *m, s);
    Some(presets::strip_folder_fields(presets::strip_run_fields(m.save(), m.run_fields())))
}

/// A stored built-in differs from the shipped one: other name/type, or values
/// that resolve to different form values (so concrete-but-default values from
/// older versions don't count as edits).
pub fn builtin_differs(p: &Preset, s: &Settings) -> bool {
    let Some(f) = p.builtin.as_deref().and_then(presets::factory) else { return false };
    p.name != f.name || p.kind != f.kind || (!p.values.is_null() && canonical_values(p, s) != canonical_values(&f, s))
}

pub fn apply_app_folders(mode: &mut dyn Mode, s: &Settings) {
    // Folder-based presets keep their own fields (blank on a fresh form; recents
    // remember last used). App-level default folders stay as set in Settings.
    if mode.wants_folders() {
        return;
    }
    let mut v = mode.save();
    let Some(obj) = v.as_object_mut() else { return };
    let mut changed = false;
    if obj.contains_key("out_dir") && obj["out_dir"] != Value::String(s.output_folder.clone()) {
        obj.insert("out_dir".into(), Value::String(s.output_folder.clone()));
        changed = true;
    }
    if obj.contains_key("in_dir") && obj["in_dir"] != Value::String(s.input_folder.clone()) {
        obj.insert("in_dir".into(), Value::String(s.input_folder.clone()));
        changed = true;
    }
    if obj.contains_key("src_dir") && obj["src_dir"] != Value::String(s.input_folder.clone()) {
        obj.insert("src_dir".into(), Value::String(s.input_folder.clone()));
        changed = true;
    }
    if changed {
        mode.load(&v, s);
    }
}

pub fn take_app_folders(mode: &dyn Mode) -> (Option<String>, Option<String>) {
    if mode.wants_folders() {
        return (None, None);
    }
    let v = mode.save();
    let out = v.get("out_dir").and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let inp = v.get("in_dir").or_else(|| v.get("src_dir")).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    (inp, out)
}

/// Folder paths currently set on a folder-based form (to remember as recents).
pub fn used_folders(mode: &dyn Mode) -> Vec<String> {
    let v = mode.save();
    let mut out = Vec::new();
    for k in ["src_dir", "in_dir", "out_dir"] {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
            out.push(s.to_string());
        }
    }
    if mode.wants_folders() {
        if let Some(s) = v.get("input").and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
            if !out.iter().any(|p| ffenc::recent_folders::folder_key(p) == ffenc::recent_folders::folder_key(s)) {
                out.push(s.to_string());
            }
        }
    }
    out
}

pub fn make_editor(p: &Preset, s: &Settings, draft: Option<&Value>) -> Option<Editor> {
    let mut mode = new_mode(&p.kind)?;
    mode.load(&p.values, s);
    apply_app_folders(&mut *mode, s);
    let baseline = presets::strip_folder_fields(presets::strip_run_fields(mode.save(), mode.run_fields()));
    if let Some(d) = draft {
        let merged = presets::overlay(mode.save(), d);
        mode.load(&merged, s);
        apply_app_folders(&mut *mode, s);
    }
    Some(Editor { mode, baseline })
}

/// egui id of the sidebar search box (Ctrl+F focuses it).
pub const SEARCH_ID: &str = "preset_search";

/// Keys that hold "the" input of a form, used to carry a file across presets.
const PRIMARY_KEYS: &[&str] = &["input", "video", "dest", "image", "signpost"];

pub fn primary_input(e: &Editor) -> Option<PathBuf> {
    let v = e.mode.save();
    // "input" is always a path; the other keys are paths only in some types
    // (Custom pipeline uses "video" for its video action, e.g. "Copy").
    PRIMARY_KEYS
        .iter()
        .filter_map(|k| v.get(*k).and_then(|x| x.as_str()).map(|s| (*k, s.trim())))
        .find(|(k, s)| !s.is_empty() && (*k == "input" || s.contains(['\\', '/'])))
        .map(|(_, s)| PathBuf::from(s))
        // Join: the first file of its list
        .or_else(|| v.get("files").and_then(|f| f.as_array()).and_then(|a| a.first()).and_then(|x| x.as_str()).map(PathBuf::from))
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

/// What to do with unsaved changes to a built-in when switching away.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaveChoice {
    SaveAs(String),
    Discard,
    /// Keep them as unsaved changes (the form remembers them).
    Keep,
}

pub enum Dialog {
    New { name: String, from: NewFrom },
    /// Switching away from a built-in with unsaved changes.
    Leave { to_id: String, run: bool, name: String },
    RestoreBuiltins,
    SaveAs { name: String },
    Rename { id: String, name: String },
    Delete { id: String },
    Reset { id: String },
    Category { id: String, category: String },
}

pub enum Action {
    None,
    Leave { to_id: String, run: bool, choice: LeaveChoice },
    RestoreBuiltins,
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
    /// Set by a confirmed "leave" prompt that came from a double-click / Run.
    pub run_after: bool,
}


/// Sidebar heading for built-in presets (including the Experimental codecs folder).
pub const DEFAULT_PRESETS: &str = "Default Presets";
/// Sidebar heading for saved, duplicated and imported presets.
pub const USER_PRESETS: &str = "User Presets";
/// Shown in User Presets when the user has none yet.
pub const USER_PRESETS_HINT: &str = "Your saved presets appear here. Use Save as new preset, or right-click a preset › Duplicate.";

/// A folder inside Default Presets or User Presets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarFolder {
    pub name: String,
    pub items: Vec<usize>,
}

/// One top-level sidebar block: Favorites, Default Presets, or User Presets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarBlock {
    pub heading: String,
    /// Uncategorized presets in this block (or every favorite, in Favorites).
    pub items: Vec<usize>,
    /// Subfolders, in order of first appearance. Experimental codecs is last in Default Presets.
    pub folders: Vec<SidebarFolder>,
    /// Plain-words hint when the block is empty (User Presets on first run).
    pub empty_hint: Option<&'static str>,
}

impl SidebarBlock {
    /// Every visible index in this block, folders included, in display order.
    pub fn all_items(&self) -> Vec<usize> {
        let mut v = self.items.clone();
        for f in &self.folders {
            v.extend(&f.items);
        }
        v
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.folders.is_empty()
    }
    pub fn count(&self) -> usize {
        self.all_items().len()
    }
}

/// Split unpinned, visible presets into uncategorized items and named folders.
fn folders_and_rest<'a>(iter: impl Iterator<Item = (usize, &'a Preset)>) -> (Vec<usize>, Vec<(String, Vec<usize>)>) {
    let mut folders: Vec<(String, Vec<usize>)> = vec![];
    let mut rest = vec![];
    for (i, p) in iter {
        let c = p.category.trim();
        if c.is_empty() {
            rest.push(i);
        } else if let Some(g) = folders.iter_mut().find(|g| g.0.eq_ignore_ascii_case(c)) {
            g.1.push(i);
        } else {
            folders.push((c.to_string(), vec![i]));
        }
    }
    (rest, folders)
}

fn take_experimental(folders: &mut Vec<(String, Vec<usize>)>) -> Option<(String, Vec<usize>)> {
    folders.iter().position(|g| g.0.eq_ignore_ascii_case(presets::EXPERIMENTAL_GROUP)).map(|i| folders.remove(i))
}

fn to_folders(v: Vec<(String, Vec<usize>)>) -> Vec<SidebarFolder> {
    v.into_iter().map(|(name, items)| SidebarFolder { name, items }).collect()
}

impl PresetUi {
    pub fn new(list: Vec<Preset>, current: usize) -> Self {
        let mut pu = PresetUi { list, current, editors: HashMap::new(), dialog: None, filter: String::new(), run_after: false };
        if pu.current >= pu.list.len() || pu.list[pu.current].hidden {
            pu.current = pu.first_visible();
        }
        pu
    }
    /// First preset shown in the sidebar (0 if everything is hidden).
    pub fn first_visible(&self) -> usize {
        self.list.iter().position(|p| !p.hidden).unwrap_or(0)
    }
    pub fn is_builtin(&self, idx: usize) -> bool {
        self.list.get(idx).map(|p| p.is_builtin()).unwrap_or(false)
    }
    /// "<name> (copy)", made unique – the suggestion in "Save as new preset".
    pub fn suggest_name(&self, idx: usize) -> String {
        presets::unique_name(&self.list, &format!("{} (copy)", self.list[idx].name))
    }
    /// Save (Ctrl+S / Save button): user presets are saved; for a built-in the
    /// "Save as new preset" dialog opens instead. Returns true when something was saved.
    pub fn save_or_prompt(&mut self, s: &Settings) -> bool {
        if self.is_builtin(self.current) {
            self.dialog = Some(Dialog::SaveAs { name: self.suggest_name(self.current) });
            false
        } else {
            self.save_current(s)
        }
    }
    pub fn hidden_count(&self) -> usize {
        self.list.iter().filter(|p| p.hidden).count()
    }
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

    /// Store the form's values into the preset. Built-ins are read-only: nothing
    /// is changed and false is returned (use Save as new preset).
    pub fn save_current(&mut self, s: &Settings) -> bool {
        let idx = self.current;
        if self.is_builtin(idx) {
            return false;
        }
        let e = self.editor(idx, s);
        let v = e.values();
        e.baseline = v.clone();
        self.list[idx].values = v;
        true
    }

    /// File › New: a fresh form for the selected preset, exactly as the preset defines it
    /// (input/output files cleared, unsaved option edits dropped). Folder-based presets
    /// start with blank folder fields; single-file presets still take output folders from
    /// Settings. Only this form is replaced: the stored presets, other forms, settings and
    /// the queue are untouched.
    pub fn new_job(&mut self, s: &Settings) {
        let p = self.list[self.current].clone();
        let e = make_editor(&p, s, None).or_else(|| make_editor(&Preset { kind: "custom".into(), ..p.clone() }, s, None)).expect("custom kind exists");
        self.editors.insert(p.id, e);
    }

    /// Keep every open form's input/output folders in line with the app-level settings.
    pub fn apply_app_folders_all(&mut self, s: &Settings) {
        for e in self.editors.values_mut() {
            apply_app_folders(&mut *e.mode, s);
        }
    }

    /// Throw away edits (keeps the files of the current run).
    pub fn revert_current(&mut self, s: &Settings) {
        let idx = self.current;
        let e = self.editor(idx, s);
        let merged = presets::overlay(e.mode.save(), &e.baseline.clone());
        e.mode.load(&merged, s);
        apply_app_folders(&mut *e.mode, s);
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

    /// Group key used for sidebar grouping and moving: favorites, then Default vs User, then folder.
    fn group_of(p: &Preset) -> String {
        if p.hidden {
            "\u{2}hidden".into()
        } else if p.pinned {
            "\u{1}fav".into()
        } else if p.is_builtin() {
            format!("\u{3}default\u{1}{}", p.category.trim())
        } else {
            format!("\u{4}user\u{1}{}", p.category.trim())
        }
    }

    /// Sidebar blocks in display order. Favorites first (when any are pinned), then
    /// Default Presets (built-ins, with Experimental codecs last among its folders),
    /// then User Presets (always shown; empty on first run). A non-empty `filter`
    /// gives one flat list of matches (heading empty).
    pub fn sidebar_blocks(&self, filter: &str) -> Vec<SidebarBlock> {
        let f = filter.trim().to_lowercase();
        if !f.is_empty() {
            let kl = kinds();
            let hits: Vec<usize> = (0..self.list.len())
                .filter(|&i| !self.list[i].hidden)
                .filter(|&i| {
                    let p = &self.list[i];
                    let kind = kl.iter().find(|k| k.0 == p.kind).map(|k| k.1).unwrap_or("");
                    [p.name.as_str(), kind, p.category.as_str(), p.description.as_str()].iter().any(|s| s.to_lowercase().contains(&f))
                })
                .collect();
            let empty_hint = hits.is_empty().then_some("No preset matches.");
            return vec![SidebarBlock { heading: String::new(), items: hits, folders: vec![], empty_hint }];
        }
        let fav: Vec<usize> = (0..self.list.len()).filter(|&i| self.list[i].pinned && !self.list[i].hidden).collect();
        let visible = self.list.iter().enumerate().filter(|(_, p)| !p.pinned && !p.hidden);
        let (def_rest, mut def_folders) = folders_and_rest(visible.clone().filter(|(_, p)| p.is_builtin()));
        let exp = take_experimental(&mut def_folders);
        let mut def_folders = to_folders(def_folders);
        def_folders.extend(exp.map(|(name, items)| SidebarFolder { name, items }));
        let (user_rest, user_folders) = folders_and_rest(visible.filter(|(_, p)| !p.is_builtin()));
        let mut out = vec![];
        if !fav.is_empty() {
            out.push(SidebarBlock { heading: "Favorites".into(), items: fav, folders: vec![], empty_hint: None });
        }
        out.push(SidebarBlock { heading: DEFAULT_PRESETS.into(), items: def_rest, folders: def_folders, empty_hint: None });
        let mut user = SidebarBlock {
            heading: USER_PRESETS.into(),
            items: user_rest,
            folders: to_folders(user_folders),
            empty_hint: None,
        };
        if user.is_empty() {
            user.empty_hint = Some(USER_PRESETS_HINT);
        }
        out.push(user);
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
        // Section headings below are "Default Presets" and "User Presets".
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
                super::common::one_line_menu(ui);
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
        // button first (right to left), the field takes exactly what is left – never widens the panel
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body) + 6.0));
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, !self.filter.is_empty(), "✖", "Clear search").clicked() {
                self.filter.clear();
            }
            let r = ui.add(egui::TextEdit::singleline(&mut self.filter).id(egui::Id::new(SEARCH_ID)).hint_text("Search presets… (Ctrl+F)").desired_width(ui.available_width()));
            named(r, egui::WidgetType::TextEdit, "Search presets");
        });
        ui.separator();

        let kind_label = |k: &str| kinds().into_iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or("?");
        let blocks = self.sidebar_blocks(&self.filter.clone());
        let mut clicked = None;
        let mut run = None;
        let mut move_req: Option<(usize, isize)> = None;
        let mut pin_toggle = None;
        let filtering = !self.filter.trim().is_empty();
        egui::ScrollArea::vertical().id_salt("presets").auto_shrink([false, false]).max_height((ui.available_height() - 100.0).max(60.0)).show(ui, |ui| {
            // full-width rows with left-aligned names
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            for block in &blocks {
                let show_star = filtering || block.heading == "Favorites";
                let mut draw = |ui: &mut egui::Ui, idxs: &[usize]| {
                    for &i in idxs {
                        let p = &self.list[i];
                        let modified = self.modified(&p.id);
                        let builtin = p.is_builtin();
                        let mut name = String::new();
                        if p.pinned && show_star {
                            name.push_str("★ ");
                        }
                        name.push_str(&p.name);
                        if modified {
                            name.push_str("  *");
                        }
                        let a11y = format!("{}{}{}", p.name, if builtin { ", built-in, read-only" } else { "" }, if modified { ", unsaved changes" } else { "" });
                        let mut text = RichText::new(name);
                        if modified && i != self.current {
                            text = text.color(crate::ui::theme::c().unsaved);
                        }
                        let tip = format!(
                            "Type: {}{}{}\n{}{}\n\nClick: select · Double-click: run · Right-click: more",
                            kind_label(&p.kind),
                            if builtin { " (built-in, read-only: save changes as a new preset)" } else { "" },
                            if p.category.trim().is_empty() { String::new() } else { format!("\nFolder: {}", p.category.trim()) },
                            p.description,
                            if modified { "\n* unsaved changes" } else { "" }
                        );
                        let sel = i == self.current;
                        let row_h = ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body) + 6.0);
                        // Fixed-width padlock column on the right (reserved for every row so names line up).
                        // Don't use add_sized + SelectableLabel: add_sized centers its widget, which
                        // centered the preset names after the padlock-slot change.
                        let lock_w = ui.spacing().interact_size.y * 0.85;
                        let r = ui.allocate_response(egui::vec2(ui.available_width(), row_h), egui::Sense::click());
                        let row_rect = r.rect;
                        let r = crate::ui::common::hover_tip(r, tip);
                        let visuals = ui.style().interact_selectable(&r, sel);
                        if ui.is_rect_visible(row_rect) {
                            if sel || r.hovered() || r.highlighted() || r.has_focus() {
                                ui.painter().rect(
                                    row_rect.expand(visuals.expansion),
                                    visuals.corner_radius,
                                    visuals.weak_bg_fill,
                                    visuals.bg_stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let lock_rect = egui::Rect::from_min_size(
                                egui::pos2(row_rect.max.x - lock_w, row_rect.min.y),
                                egui::vec2(lock_w, row_h),
                            );
                            let pad = ui.spacing().button_padding;
                            let text_rect = egui::Rect::from_min_max(
                                row_rect.min,
                                egui::pos2(lock_rect.min.x, row_rect.max.y),
                            ).shrink2(pad);
                            let galley = egui::WidgetText::from(text).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                text_rect.width().max(1.0),
                                egui::TextStyle::Button,
                            );
                            let text_pos = egui::pos2(
                                text_rect.left(),
                                text_rect.center().y - galley.size().y / 2.0,
                            );
                            ui.painter().galley(text_pos, galley, visuals.text_color());
                            if builtin {
                                let lock = ui.allocate_new_ui(
                                    egui::UiBuilder::new()
                                        .id_salt(("preset_lock", i))
                                        .max_rect(lock_rect)
                                        .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
                                    |ui| ui.add(egui::Label::new("🔒").selectable(false)),
                                ).inner;
                                let _ = lock.on_hover_text("Built-in, read-only: save changes as a new preset");
                            }
                        }
                        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, sel, &a11y));
                        if r.double_clicked() {
                            run = Some(i);
                        } else if r.clicked() {
                            clicked = Some(i);
                        }
                        let id = p.id.clone();
                        let (pinned, cat) = (p.pinned, p.category.clone());
                        r.context_menu(|ui| {
                            super::common::one_line_menu(ui);
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
                            if ui.add_enabled(!builtin, egui::Button::new("Rename…")).on_disabled_hover_text("Built-in presets can't be renamed – duplicate it instead.").clicked() {
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
                            if builtin && ui.add_enabled(modified, egui::Button::new("Discard unsaved changes…")).clicked() {
                                self.dialog = Some(Dialog::Reset { id: id.clone() });
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(if builtin { "Hide…" } else { "Delete…" }).clicked() {
                                self.dialog = Some(Dialog::Delete { id: id.clone() });
                                ui.close_menu();
                            }
                        });
                    }
                };
                if filtering || block.heading.is_empty() {
                    if let Some(h) = block.empty_hint {
                        ui.label(RichText::new(h).color(crate::ui::theme::c().muted));
                    }
                    draw(ui, &block.items);
                } else {
                    let n = block.count();
                    let title = if block.heading == "Favorites" {
                        format!("★ Favorites ({n})")
                    } else if n == 0 {
                        block.heading.clone()
                    } else {
                        format!("{} ({n})", block.heading)
                    };
                    crate::ui::layout::section(ui, &format!("presets.group:{}", block.heading), egui::CollapsingHeader::new(RichText::new(title).strong()), true, |ui| {
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                            if let Some(h) = block.empty_hint {
                                ui.label(RichText::new(h).color(crate::ui::theme::c().muted));
                            }
                            draw(ui, &block.items);
                            for folder in &block.folders {
                                let ft = format!("{} ({})", folder.name, folder.items.len());
                                crate::ui::layout::section(
                                    ui,
                                    &format!("presets.group:{}:{}", block.heading, folder.name),
                                    egui::CollapsingHeader::new(RichText::new(ft).strong()),
                                    true,
                                    |ui| ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| draw(ui, &folder.items)),
                                );
                            }
                        });
                    });
                }
            }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            let i = self.current;
            if icon_button(ui, self.can_move(i, -1), "⬆", "Move selected preset up (within its group)").clicked() {
                move_req = Some((i, -1));
            }
            if icon_button(ui, self.can_move(i, 1), "⬇", "Move selected preset down (within its group)").clicked() {
                move_req = Some((i, 1));
            }
            let pinned = self.list[i].pinned;
            if named(ui.small_button(if pinned { "Unpin" } else { "Pin" }), egui::WidgetType::Button, if pinned { "Unpin selected preset (remove from Favorites)" } else { "Pin selected preset to Favorites (listed at the top)" }).clicked() {
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
            if self.ask_before_leaving(i, true) {
                return ev;
            }
            self.select(i, cx);
            return Some(SideEvent::Run);
        }
        if let Some(i) = clicked {
            if self.ask_before_leaving(i, false) {
                return ev;
            }
            self.select(i, cx);
            ev = ev.or(Some(SideEvent::Selected));
        }
        ev
    }

    /// Switching from a built-in with unsaved changes to preset `to`: opens the
    /// "Save as new / Discard / Keep" prompt and returns true (the switch waits).
    pub fn ask_before_leaving(&mut self, to: usize, run: bool) -> bool {
        if to == self.current || to >= self.list.len() || !self.is_builtin(self.current) || !self.modified(&self.cur().id.clone()) {
            return false;
        }
        self.dialog = Some(Dialog::Leave { to_id: self.list[to].id.clone(), run, name: self.suggest_name(self.current) });
        true
    }

    /// Draw the open dialog; returns what the user confirmed.
    pub fn dialogs(&mut self, ctx: &egui::Context) -> Action {
        let Some(d) = self.dialog.as_mut() else { return Action::None };
        let mut act = Action::None;
        let mut close = false;
        let names: Vec<(String, String)> = self.list.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        let center = egui::Align2::CENTER_CENTER;
        let cur_name = self.list[self.current].name.clone();
        match d {
            Dialog::Leave { to_id, run, name } => {
                crate::ui::common::fit(egui::Window::new("Unsaved changes to a built-in preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    ui.label(format!("You changed “{cur_name}”. Built-in presets are read-only, so the changes can't be saved into it."));
                    ui.label("Save them as a new preset?");
                    let l = ui.label("New preset name");
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                    r.clone().labelled_by(l.id);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !enter && ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(RichText::new("Save as new preset").strong()).clicked() || enter {
                            act = Action::Leave { to_id: to_id.clone(), run: *run, choice: LeaveChoice::SaveAs(name.clone()) };
                            close = true;
                        }
                        if ui.button("Discard changes").clicked() {
                            act = Action::Leave { to_id: to_id.clone(), run: *run, choice: LeaveChoice::Discard };
                            close = true;
                        }
                        if named(ui.button("Keep for later"), egui::WidgetType::Button, "Keep for later: switch now and keep the changes as unsaved changes in this form").clicked() {
                            act = Action::Leave { to_id: to_id.clone(), run: *run, choice: LeaveChoice::Keep };
                            close = true;
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            close = true;
                        }
                    });
                    hint(ui, "“Keep for later” switches now; the built-in keeps showing your changes as unsaved (*) until you save them as a new preset or discard them.");
                });
            }
            Dialog::RestoreBuiltins => {
                let hidden = self.list.iter().filter(|p| p.hidden).count();
                let edited = self.list.iter().filter(|p| p.is_builtin() && self.editors.get(&p.id).map(|e| e.modified()).unwrap_or(false)).count();
                crate::ui::common::fit(egui::Window::new("Restore built-in presets").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    ui.label(format!("Show every built-in preset again ({hidden} hidden) and throw away unsaved changes in built-ins ({edited})?"));
                    hint(ui, "Built-in presets always have the values that ship with the app. Your own presets are not touched.");
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Restore").strong()).clicked() {
                            act = Action::RestoreBuiltins;
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::New { name, from } => {
                crate::ui::common::fit(egui::Window::new("New preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    egui::Grid::new("np").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                        crate::ui::common::row_label(ui, "Start from");
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
                        crate::ui::common::row_label(ui, "Name");
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
                let builtin = self.list[self.current].is_builtin();
                crate::ui::common::fit(egui::Window::new("Save as new preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    if builtin {
                        ui.label(format!("“{cur_name}” is a built-in preset and is read-only, so your changes are saved as a new preset."));
                    }
                    ui.label("Saves the current form as a new preset. The original preset keeps its saved values.");
                    let l = ui.label("New preset name");
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                    r.clone().labelled_by(l.id);
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
                crate::ui::common::fit(egui::Window::new("Rename preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
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
            Dialog::Delete { id } if self.list.iter().any(|p| &p.id == id && p.is_builtin()) => {
                let n = names.iter().find(|n| &n.0 == id).map(|n| n.1.clone()).unwrap_or_default();
                crate::ui::common::fit(egui::Window::new("Hide built-in preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    ui.label(format!("Hide the built-in preset “{n}” from the list?"));
                    hint(ui, "Built-in presets can't be deleted. Bring it back any time with Presets › Restore built-in presets.");
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Hide").strong()).clicked() {
                            act = Action::Delete { id: id.clone() };
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
                crate::ui::common::fit(egui::Window::new("Delete preset").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    ui.label(format!("Delete the preset “{n}”?"));
                    hint(ui, "This can't be undone. Export it first if you want a copy.");
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Delete").color(crate::ui::theme::c().err)).clicked() {
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
                crate::ui::common::fit(egui::Window::new("Preset folder").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
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
                crate::ui::common::fit(egui::Window::new("Discard unsaved changes").collapsible(false).resizable(false).anchor(center, [0.0, 0.0]), ctx, true).show(ctx, |ui| {
                    ui.label(format!("Throw away your unsaved changes to “{n}” and go back to the built-in values?"));
                    hint(ui, "Built-in values follow File › Settings defaults (CRF, preset, bitrate, …). The chosen files are kept.");
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Discard changes").strong()).clicked() {
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
            Action::Leave { to_id, run, choice } => {
                match choice {
                    LeaveChoice::SaveAs(name) => self.apply(Action::SaveAs { name }, cx)?,
                    LeaveChoice::Discard => self.revert_current(s),
                    LeaveChoice::Keep => {}
                }
                if let Some(i) = self.index_of(&to_id) {
                    self.select(i, cx);
                }
                self.run_after = run;
            }
            Action::RestoreBuiltins => {
                for i in 0..self.list.len() {
                    if self.list[i].is_builtin() {
                        self.list[i].hidden = false;
                        let id = self.list[i].id.clone();
                        if let Some(e) = self.editors.get(&id) {
                            let run = e.run_values();
                            if let Some(ne) = make_editor(&self.list[i], s, Some(&run)) {
                                self.editors.insert(id, ne);
                            }
                        }
                    }
                }
            }
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
                let p = Preset { id: presets::new_id(), name: presets::unique_name(&self.list, &name), kind, description: desc, builtin: None, values, pinned: false, category, hidden: false };
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
                let p = Preset { id: presets::new_id(), name: presets::unique_name(&self.list, &name), kind: src.kind.clone(), description: src.description.clone(), builtin: None, values, pinned: false, category: src.category.clone(), hidden: false };
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
                if self.list.iter().any(|p| p.id == id && p.is_builtin()) {
                    return Err("Built-in presets can't be renamed. Use Duplicate (or Save as new preset) to make your own copy.".into());
                }
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
                let Some(i) = self.index_of(&id) else { return Ok(()) };
                if self.list.iter().filter(|p| !p.hidden).count() <= 1 && !self.list[i].hidden {
                    return Err("Keep at least one preset in the list.".into());
                }
                if self.list[i].is_builtin() {
                    // built-ins are hidden, never deleted
                    self.list[i].hidden = true;
                    if self.current == i {
                        let next = (i + 1..self.list.len()).chain(0..i).find(|&j| !self.list[j].hidden).unwrap_or(0);
                        self.select(next, cx);
                    }
                    return Ok(());
                }
                self.list.remove(i);
                self.editors.remove(&id);
                if self.current >= self.list.len() || self.current > i {
                    self.current = self.current.saturating_sub(1).min(self.list.len() - 1);
                }
                if self.list[self.current].hidden {
                    self.current = self.first_visible();
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
                let (pinned, category, hidden) = (self.list[i].pinned, self.list[i].category.clone(), self.list[i].hidden);
                self.list[i] = Preset { id: id.clone(), pinned, category, hidden, ..f };
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
        PresetUi { list, current: 0, editors: HashMap::new(), dialog: None, filter: String::new(), run_after: false }
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
    fn file_new_gives_a_fresh_form_and_keeps_everything_else() {
        let s = Settings { output_folder: "/my/out".into(), input_folder: "/my/in".into(), crf: 19, ..Settings::default() };
        let s_before = serde_json::to_value(&s).unwrap();
        let mut pu = ui_with(presets::factory_all());
        // a user preset with its own saved values
        let mine = Preset { id: "pmine".into(), name: "Mine".into(), builtin: None, values: json!({"trim_start": 3.0}), ..pu.list[0].clone() };
        pu.list.push(mine);
        let fresh_builtin = make_editor(&pu.list[0].clone(), &s, None).unwrap().mode.save();
        // another form with a file that must survive
        set(pu.editor(1, &s), "video", json!("/v/other.mkv"), &s);
        let other = pu.editor(1, &s).mode.save();
        // the finished job's form: file, output name and an option edit
        set(pu.editor(0, &s), "input", json!("/v/a.mkv"), &s);
        set(pu.editor(0, &s), "trim_start", json!(12.5), &s);
        assert!(pu.modified("builtin-reencode"));
        let list_before = serde_json::to_value(&pu.list).unwrap();
        pu.new_job(&s);
        let v = pu.editor(0, &s).mode.save();
        assert_eq!(v, fresh_builtin, "same as a freshly opened preset");
        assert_eq!(v["input"], json!(""));
        assert_eq!(v["out_dir"], json!("/my/out"), "output folder preference kept");
        assert_eq!(v["video"]["crf"], json!(19), "Settings defaults kept");
        assert!(!pu.modified("builtin-reencode"));
        assert!(pu.drafts().get("builtin-reencode").is_none(), "nothing left to persist as a draft");
        assert_eq!(serde_json::to_value(&pu.list).unwrap(), list_before, "stored presets untouched");
        assert_eq!(pu.editor(1, &s).mode.save(), other, "other forms untouched");
        assert_eq!(serde_json::to_value(&s).unwrap(), s_before, "settings untouched");
        // a user preset comes back with its own saved values
        pu.current = pu.list.len() - 1;
        let i = pu.current;
        set(pu.editor(i, &s), "input", json!("/v/b.mkv"), &s);
        pu.new_job(&s);
        let v = pu.editor(i, &s).mode.save();
        assert_eq!((v["trim_start"].clone(), v["input"].clone()), (json!(3.0), json!("")));
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
        // built-ins are read-only: Save changes nothing
        set(pu.editor(0, &s), "trim_start", json!(3.0), &s);
        assert!(!pu.save_current(&s));
        assert!(pu.list[0].values.is_null());
        assert!(pu.modified("builtin-reencode"), "the edit stays as an unsaved change");
        // a user preset saves normally (values only, no run fields)
        let mine = Preset { id: "pmine".into(), name: "Mine".into(), builtin: None, ..pu.list[0].clone() };
        pu.list.push(mine);
        pu.current = pu.list.len() - 1;
        let i = pu.current;
        set(pu.editor(i, &s), "input", json!("/v/a.mkv"), &s);
        set(pu.editor(i, &s), "trim_start", json!(3.0), &s);
        assert!(pu.save_or_prompt(&s));
        assert!(pu.dialog.is_none());
        assert!(!pu.modified("pmine"));
        assert_eq!(pu.list[i].values["trim_start"], json!(3.0));
        assert!(pu.list[i].values.get("input").is_none());
    }

    #[test]
    fn builtins_are_read_only_and_prompt_save_as_new() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx, has_xfade: None, encoders: None, ff: None, managed_encoders: None, switch_to_managed: None };
        let mut pu = ui_with(presets::factory_all());
        assert!(pu.is_builtin(0));
        set(pu.editor(0, &s), "input", json!("/v/a.mkv"), &s);
        set(pu.editor(0, &s), "trim_end", json!(9.0), &s);
        // Ctrl+S / Save on a built-in opens "Save as new preset" with a free name
        assert!(!pu.save_or_prompt(&s));
        let Some(Dialog::SaveAs { name }) = &pu.dialog else { panic!("no save-as prompt") };
        assert_eq!(name, "Re-encode video (copy)");
        pu.apply(Action::SaveAs { name: name.clone() }, &cx).unwrap();
        pu.dialog = None;
        assert_eq!(pu.cur().name, "Re-encode video (copy)");
        assert!(!pu.cur().is_builtin() && pu.cur().values["trim_end"] == json!(9.0));
        assert!(pu.list[0].values.is_null() && !pu.modified("builtin-reencode"), "built-in untouched and reverted");
        // name collision: the next suggestion is unique
        assert_eq!(pu.suggest_name(0), "Re-encode video (copy) (2)");
        // rename refused, delete hides
        assert!(pu.apply(Action::Rename { id: "builtin-mix".into(), name: "X".into() }, &cx).is_err());
        let mix = pu.index_of("builtin-mix").unwrap();
        pu.current = mix;
        pu.apply(Action::Delete { id: "builtin-mix".into() }, &cx).unwrap();
        assert!(pu.index_of("builtin-mix").is_some() && pu.list[mix].hidden, "hidden, not deleted");
        assert_ne!(pu.current, mix, "selection moved to a visible preset");
        let shown = |q: &str| pu.sidebar_blocks(q).iter().flat_map(|b| b.all_items()).collect::<Vec<_>>();
        assert!(!shown("").contains(&mix) && !shown("mix").contains(&mix));
        assert_eq!(pu.hidden_count(), 1);
        // restore: visible again, unsaved built-in edits dropped
        set(pu.editor(0, &s), "trim_end", json!(5.0), &s);
        pu.apply(Action::RestoreBuiltins, &cx).unwrap();
        assert_eq!(pu.hidden_count(), 0);
        assert!(!pu.modified("builtin-reencode"));
        assert_eq!(pu.editor(0, &s).mode.save()["input"], json!("/v/a.mkv"), "files kept");
        // user presets: delete really deletes; can't hide the last visible preset
        let id = pu.list[1].id.clone();
        assert!(!pu.list[1].is_builtin());
        pu.apply(Action::Delete { id: id.clone() }, &cx).unwrap();
        assert!(pu.index_of(&id).is_none());
        let mut one = ui_with(vec![presets::factory("builtin-mix").unwrap()]);
        assert!(one.apply(Action::Delete { id: "builtin-mix".into() }, &cx).is_err());
    }

    #[test]
    fn migration_compares_resolved_values() {
        let s = Settings::default();
        let f = presets::factory("builtin-reencode").unwrap();
        assert!(!builtin_differs(&f, &s));
        // concrete values equal to the defaults (saved by 0.4 without real edits)
        let same = Preset { values: canonical_values(&f, &s).unwrap(), ..f.clone() };
        assert!(!builtin_differs(&same, &s));
        let mut v = canonical_values(&f, &s).unwrap();
        v["video"]["crf"] = json!(17);
        assert!(builtin_differs(&Preset { values: v, ..f.clone() }, &s));
        assert!(builtin_differs(&Preset { name: "Mine".into(), ..f.clone() }, &s));
        let user = Preset { builtin: None, name: "x".into(), ..f };
        assert!(!builtin_differs(&user, &s));
    }

    #[test]
    fn leaving_an_edited_builtin_prompts() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx, has_xfade: None, encoders: None, ff: None, managed_encoders: None, switch_to_managed: None };
        let mut pu = ui_with(presets::factory_all());
        let mix = pu.index_of("builtin-mix").unwrap();
        // unchanged: no prompt
        assert!(!pu.ask_before_leaving(mix, false));
        set(pu.editor(0, &s), "trim_start", json!(2.0), &s);
        assert!(!pu.ask_before_leaving(0, false), "same preset");
        assert!(pu.ask_before_leaving(mix, true));
        let Some(Dialog::Leave { to_id, run, name }) = &pu.dialog else { panic!() };
        assert_eq!((to_id.as_str(), *run, name.as_str()), ("builtin-mix", true, "Re-encode video (copy)"));
        // keep for later: switch, edits stay unsaved
        pu.apply(Action::Leave { to_id: "builtin-mix".into(), run: false, choice: LeaveChoice::Keep }, &cx).unwrap();
        assert_eq!(pu.cur().id, "builtin-mix");
        assert!(pu.modified("builtin-reencode"));
        // discard
        pu.current = 0;
        pu.apply(Action::Leave { to_id: "builtin-mix".into(), run: true, choice: LeaveChoice::Discard }, &cx).unwrap();
        assert!(!pu.modified("builtin-reencode") && pu.cur().id == "builtin-mix" && pu.run_after);
        // save as new, then switch to the requested preset (index shifts are handled by id)
        pu.run_after = false;
        pu.current = 0;
        set(pu.editor(0, &s), "trim_start", json!(4.0), &s);
        pu.apply(Action::Leave { to_id: "builtin-mix".into(), run: false, choice: LeaveChoice::SaveAs("Short start".into()) }, &cx).unwrap();
        assert_eq!(pu.cur().id, "builtin-mix");
        let new = pu.list.iter().find(|p| p.name == "Short start").unwrap();
        assert_eq!(new.values["trim_start"], json!(4.0));
        assert!(!pu.modified("builtin-reencode") && pu.list[0].values.is_null());
        // a user preset with changes never prompts
        let i = pu.list.iter().position(|p| p.name == "Short start").unwrap();
        pu.current = i;
        set(pu.editor(i, &s), "trim_start", json!(1.0), &s);
        assert!(!pu.ask_before_leaving(0, false));
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
        let still = pu.list.iter().position(|p| p.id == "builtin-still").unwrap();
        set(pu.editor(still, &s), "custom_duration", json!(99.0), &s);
        let s2 = Settings { crf: 30, audio_bitrate_k: 128, ..Settings::default() };
        pu.rebase(&s2);
        assert_eq!(pu.editor(0, &s2).mode.save()["video"]["crf"], json!(30));
        assert!(!pu.modified("builtin-reencode"));
        assert!(pu.modified("builtin-still"), "edits survive");
        assert_eq!(pu.editor(still, &s2).mode.save()["custom_duration"], json!(99.0));
    }

    #[test]
    fn create_saveas_rename_delete_reset() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx, has_xfade: None, encoders: None, ff: None, managed_encoders: None, switch_to_managed: None };
        let mut pu = ui_with(presets::factory_all());
        let n0 = pu.list.len();
        // blank custom
        pu.apply(Action::Create { name: "".into(), from: NewFrom::Kind("custom".into()) }, &cx).unwrap();
        assert_eq!(pu.list.len(), n0 + 1);
        assert_eq!(pu.current, 1);
        assert_eq!(pu.cur().kind, "custom");
        assert_eq!(pu.cur().name, "Custom pipeline (2)");
        assert!(pu.cur().builtin.is_none());
        // copy of re-encode: the built-in's saved values (factory), not unsaved edits
        pu.current = 0;
        set(pu.editor(0, &s), "trim_start", json!(7.0), &s);
        assert!(!pu.save_current(&s));
        pu.apply(Action::Create { name: "My encode".into(), from: NewFrom::Copy("builtin-reencode".into()) }, &cx).unwrap();
        assert_eq!(pu.cur().name, "My encode");
        assert_eq!(pu.cur().values["trim_start"], json!(0.0));
        pu.revert_current(&s);
        let me = pu.current;
        pu.current = 0;
        pu.revert_current(&s);
        pu.current = me;
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
        // reset (discard changes) of a built-in
        assert!(pu.apply(Action::Rename { id: "builtin-reencode".into(), name: "Mine".into() }, &cx).is_err());
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
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx, has_xfade: None, encoders: None, ff: None, managed_encoders: None, switch_to_managed: None };
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
    fn folders_are_app_level_ignored_on_presets_and_kept_across_new_and_switch() {
        let s = Settings { output_folder: "/app/out".into(), input_folder: "/app/in".into(), ..Settings::default() };
        let mut pu = ui_with(presets::factory_all());
        // a user preset that still has baked-in folder fields (older save)
        let mine = Preset {
            id: "pmine".into(),
            name: "Mine".into(),
            builtin: None,
            values: json!({"trim_start": 3.0, "out_dir": "/old/out", "in_dir": "/old/in"}),
            ..pu.list[0].clone()
        };
        pu.list.push(mine);
        let v = make_editor(&pu.list[pu.list.len() - 1], &s, None).unwrap().mode.save();
        assert_eq!(v["out_dir"], json!("/app/out"), "saved preset out_dir is ignored");
        assert_eq!(v["trim_start"], json!(3.0), "encode settings still apply");

        // File > New keeps the remembered folders, not the preset's
        pu.current = pu.list.len() - 1;
        set(pu.editor(pu.current, &s), "input", json!("/v/a.mkv"), &s);
        set(pu.editor(pu.current, &s), "out_dir", json!("/should-not-stick"), &s);
        // changing the folder is not a preset edit
        // (re-apply app folders first like the live form does after a settings sync)
        apply_app_folders(&mut *pu.editor(pu.current, &s).mode, &s);
        pu.new_job(&s);
        let v = pu.editor(pu.current, &s).mode.save();
        assert_eq!(v["out_dir"], json!("/app/out"));
        assert_eq!(v["input"], json!(""));
        assert_eq!(v["trim_start"], json!(3.0));

        // switching presets keeps the app folders
        let other = make_editor(&pu.list[1], &s, None).unwrap().mode.save();
        assert_eq!(other["out_dir"], json!("/app/out"));

        // export omits folder fields
        let exp = pu.export_copy(pu.current, &s);
        assert!(exp.values.get("out_dir").is_none(), "export must omit out_dir, got {}", exp.values);
        assert!(exp.values.get("in_dir").is_none());
        assert_eq!(exp.values["trim_start"], json!(3.0));

        // changing output folder does not dirty the preset
        let id0 = pu.list[0].id.clone();
        assert!(!pu.modified(&id0));
        set(pu.editor(0, &s), "out_dir", json!("/other/out"), &s);
        assert!(!pu.modified(&id0), "folder is not a preset field");
    }

    #[test]
    fn file_new_leaves_folder_presets_blank_and_single_file_unchanged() {
        let s = Settings { output_folder: "/app/out".into(), input_folder: "/app/in".into(), ..Settings::default() };
        let mut pu = ui_with(presets::factory_all());
        let remux = pu.index_of("builtin-remux").expect("remux");
        let batch = pu.index_of("builtin-batch").expect("batch");
        assert_eq!(pu.list[0].kind, "reencode");

        let rv = make_editor(&pu.list[remux], &s, None).unwrap().mode.save();
        assert_eq!(rv["src_dir"], json!(""));
        assert_eq!(rv["out_dir"], json!(""));
        let bv = make_editor(&pu.list[batch], &s, None).unwrap().mode.save();
        assert_eq!(bv["in_dir"], json!(""));
        assert_eq!(bv["out_dir"], json!(""));

        pu.current = remux;
        set(pu.editor(remux, &s), "src_dir", json!("/picked/in"), &s);
        set(pu.editor(remux, &s), "out_dir", json!("/picked/out"), &s);
        assert_eq!(take_app_folders(&*pu.editor(remux, &s).mode), (None, None));
        apply_app_folders(&mut *pu.editor(remux, &s).mode, &s);
        let v = pu.editor(remux, &s).mode.save();
        assert_eq!(v["src_dir"], json!("/picked/in"), "apply_app_folders must not overwrite Remux folders");
        assert_eq!(v["out_dir"], json!("/picked/out"));

        pu.new_job(&s);
        let v = pu.editor(remux, &s).mode.save();
        assert_eq!(v["src_dir"], json!(""));
        assert_eq!(v["out_dir"], json!(""));

        // single-file presets still take the Settings output folder
        let ev = make_editor(&pu.list[0], &s, None).unwrap().mode.save();
        assert_eq!(ev["out_dir"], json!("/app/out"));
        pu.current = 0;
        set(pu.editor(0, &s), "input", json!("/v/a.mkv"), &s);
        set(pu.editor(0, &s), "out_dir", json!("/should-reset"), &s);
        apply_app_folders(&mut *pu.editor(0, &s).mode, &s);
        pu.new_job(&s);
        let v = pu.editor(0, &s).mode.save();
        assert_eq!(v["out_dir"], json!("/app/out"));
        assert_eq!(v["input"], json!(""));
        assert_eq!(take_app_folders(&*pu.editor(0, &s).mode), (None, Some("/app/out".into())));
        // Custom in folder mode starts blank; file mode still uses Settings output
        let custom = pu.index_of("builtin-custom").expect("custom");
        let mut cv = make_editor(&pu.list[custom], &s, None).unwrap().mode.save();
        cv["batch"] = json!(true);
        if let Some(obj) = cv.as_object_mut() {
            obj.remove("out_dir");
            obj.remove("input");
        }
        let batch_preset = Preset { values: cv, ..pu.list[custom].clone() };
        let bv = make_editor(&batch_preset, &s, None).unwrap().mode.save();
        assert_eq!(bv["input"], json!(""));
        assert_eq!(bv["out_dir"], json!(""));
        let fv = make_editor(&pu.list[custom], &s, None).unwrap().mode.save();
        assert_eq!(fv["out_dir"], json!("/app/out"));
    }

    #[test]
    fn export_resolves_defaults() {
        let s = Settings { crf: 19, ..Settings::default() };
        let pu = ui_with(presets::factory_all());
        let p = pu.export_copy(0, &s);
        assert_eq!(p.values["video"]["crf"], json!(19));
        assert!(p.values.get("out_dir").is_none(), "export omits output folder");
        assert!(p.values.get("in_dir").is_none());
        assert!(pu.list[0].values.is_null(), "stored preset untouched");
    }

    fn ids(pu: &PresetUi, idxs: &[usize]) -> Vec<String> {
        idxs.iter().map(|&i| pu.list[i].id.clone()).collect()
    }

    #[test]
    fn sidebar_sections_filter_and_group_moves() {
        let mut pu = ui_with(presets::factory_all());
        let n = pu.list.len();
        let blocks = pu.sidebar_blocks("");
        // built-ins in Default Presets; Experimental codecs is a folder inside it; User Presets is empty
        let e = n - 4;
        assert_eq!(blocks.iter().map(|b| b.heading.as_str()).collect::<Vec<_>>(), [DEFAULT_PRESETS, USER_PRESETS]);
        assert_eq!(blocks[0].items, (0..e).collect::<Vec<_>>(), "built-in order kept");
        assert_eq!(blocks[0].folders.len(), 1);
        assert_eq!(blocks[0].folders[0].name, presets::EXPERIMENTAL_GROUP);
        assert_eq!(blocks[0].folders[0].items, (e..n).collect::<Vec<_>>());
        assert!(blocks[0].items.iter().chain(blocks[0].folders[0].items.iter()).all(|&i| pu.list[i].is_builtin()));
        assert!(blocks[1].is_empty());
        assert_eq!(blocks[1].empty_hint, Some(USER_PRESETS_HINT));
        // favorites + a built-in folder (stays inside Default Presets)
        pu.list[3].pinned = true;
        pu.list[5].pinned = true;
        pu.list[1].category = "Audio".into();
        pu.list[4].category = "audio".into();
        let blocks = pu.sidebar_blocks("");
        let heads: Vec<_> = blocks.iter().map(|s| s.heading.as_str()).collect();
        assert_eq!(heads, ["Favorites", DEFAULT_PRESETS, USER_PRESETS]);
        assert_eq!(blocks[0].items, vec![3, 5]);
        let audio = blocks[1].folders.iter().find(|f| f.name == "Audio").expect("Audio folder in Default Presets");
        assert_eq!(audio.items, vec![1, 4], "folder names match case-insensitively");
        assert!(!blocks[1].items.contains(&3) && blocks[1].items.contains(&0));
        assert!(blocks[2].is_empty(), "builtin folders do not leak into User Presets");
        // search: name, type label, folder, description; flat
        let hit = |q: &str, pu: &PresetUi| ids(pu, &pu.sidebar_blocks(q)[0].items);
        assert_eq!(hit("REMUX", &pu), ["builtin-remux"]);
        assert!(hit("audio", &pu).len() >= 3);
        assert!(hit("custom pipeline", &pu).contains(&"builtin-addchapters".to_string()), "matches the type label");
        assert!(hit("zzz", &pu).is_empty());
        assert_eq!(pu.sidebar_blocks("zzz")[0].empty_hint, Some("No preset matches."));
        // moving inside the favorites group skips the presets between them
        pu.current = 5;
        assert!(pu.move_in_group(5, -1));
        assert_eq!(pu.sidebar_blocks("")[0].items, vec![3, 4]);
        assert!(pu.list[3].pinned && pu.list[4].pinned);
        assert_eq!(pu.current, 3, "selection follows the moved preset");
        assert!(!pu.move_in_group(3, -1), "already first in its group");
        assert!(!pu.can_move(4, 1));
    }

    #[test]
    fn sidebar_user_presets_from_old_files_and_folders() {
        let mut pu = ui_with(presets::factory_all());
        // old saved / duplicated / imported presets have builtin: None
        let base = pu.list[0].clone();
        pu.list.push(Preset { id: "u1".into(), name: "My encode".into(), builtin: None, category: String::new(), pinned: false, hidden: false, ..base.clone() });
        pu.list.push(Preset { id: "u2".into(), name: "In a folder".into(), builtin: None, category: "Projects".into(), pinned: false, hidden: false, ..base.clone() });
        pu.list.push(Preset { id: "u3".into(), name: "Second".into(), builtin: None, category: String::new(), pinned: false, hidden: false, ..base.clone() });
        let blocks = pu.sidebar_blocks("");
        let user = blocks.iter().find(|b| b.heading == USER_PRESETS).unwrap();
        assert_eq!(ids(&pu, &user.items), ["u1", "u3"], "uncategorized order kept");
        assert_eq!(user.folders.len(), 1);
        assert_eq!(user.folders[0].name, "Projects");
        assert_eq!(ids(&pu, &user.folders[0].items), ["u2"]);
        assert!(user.empty_hint.is_none());
        assert!(user.all_items().iter().all(|&i| !pu.list[i].is_builtin()));
        // a user folder is not mixed with a same-named builtin folder
        pu.list[1].category = "Projects".into();
        let blocks = pu.sidebar_blocks("");
        let def = blocks.iter().find(|b| b.heading == DEFAULT_PRESETS).unwrap();
        let user = blocks.iter().find(|b| b.heading == USER_PRESETS).unwrap();
        assert!(def.folders.iter().any(|f| f.name == "Projects" && f.items.contains(&1) && pu.list[1].is_builtin()));
        assert!(user.folders.iter().any(|f| f.name == "Projects" && ids(&pu, &f.items) == ["u2"]));
        // pinning a user preset moves it to Favorites and out of User Presets
        let u1 = pu.index_of("u1").unwrap();
        pu.list[u1].pinned = true;
        pu.list[0].pinned = true;
        let blocks = pu.sidebar_blocks("");
        assert_eq!(blocks[0].heading, "Favorites");
        assert_eq!(ids(&pu, &blocks[0].items), ["builtin-reencode", "u1"]);
        let user = blocks.iter().find(|b| b.heading == USER_PRESETS).unwrap();
        assert!(!user.all_items().iter().any(|&i| pu.list[i].id == "u1"));
        let def = blocks.iter().find(|b| b.heading == DEFAULT_PRESETS).unwrap();
        assert!(!def.all_items().iter().any(|&i| pu.list[i].id == "builtin-reencode"));
        // hidden user presets do not appear
        let u3 = pu.index_of("u3").unwrap();
        pu.list[u3].hidden = true;
        let blocks = pu.sidebar_blocks("");
        let user = blocks.iter().find(|b| b.heading == USER_PRESETS).unwrap();
        assert!(!user.all_items().iter().any(|&i| pu.list[i].id == "u3"));
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
        let cx = Ctx { settings: &s, ffprobe: None, egui: &ctx, has_xfade: None, encoders: None, ff: None, managed_encoders: None, switch_to_managed: None };
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
