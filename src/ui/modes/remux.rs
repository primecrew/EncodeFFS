//! Mode 9 – remux MKV -> MP4 (container copy, never overwrite).

use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::remux;
use ffenc::ffmpeg::runner::{JobSpec, PostAction};
use ffenc::settings::Settings;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    src_dir: String,
    /// Empty = same as the source folder (script default).
    out_dir: String,
    /// Never remembered: deleting sources must be an explicit choice each session.
    #[serde(skip)]
    delete_sources: bool,
}

#[derive(Default)]
pub struct RemuxMode {
    st: State,
    files: Vec<(PathBuf, bool)>,
    listed_dir: Option<String>,
    list_error: Option<String>,
}

fn list_mkv(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && ffenc::util::ext_lower(p) == "mkv")
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(v)
}

impl RemuxMode {
    fn refresh(&mut self) {
        self.listed_dir = Some(self.st.src_dir.clone());
        self.list_error = None;
        match list_mkv(Path::new(self.st.src_dir.trim())) {
            Ok(v) => self.files = v.into_iter().map(|p| (p, true)).collect(),
            Err(e) => {
                self.files.clear();
                self.list_error = Some(e);
            }
        }
    }
    fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            if ffenc::util::ext_lower(&p) == "mkv" && !self.files.iter().any(|(f, _)| *f == p) {
                self.files.push((p, true));
            }
        }
    }
    fn dest_dir(&self) -> PathBuf {
        if self.st.out_dir.trim().is_empty() {
            PathBuf::from(self.st.src_dir.trim())
        } else {
            PathBuf::from(self.st.out_dir.trim())
        }
    }
}

impl Mode for RemuxMode {
    fn key(&self) -> &'static str {
        "remux"
    }
    fn title(&self) -> &'static str {
        "Remux MKV -> MP4"
    }
    fn description(&self) -> &'static str {
        "Container copy (-map 0 -c copy). Never overwrites; existing MP4s are skipped."
    }
    fn run_fields(&self) -> &'static [&'static str] {
        &["src_dir"]
    }
    fn needs_encoders(&self) -> bool {
        false
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &Ctx) {
        if self.listed_dir.is_none() && PathBuf::from(self.st.src_dir.trim()).is_dir() {
            self.refresh();
        }
        let mut refresh = false;
        egui::Grid::new("m9").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Source folder", &mut self.st.src_dir, Pick::Folder, &cx.settings.input_folder) && PathBuf::from(self.st.src_dir.trim()).is_dir() {
                refresh = true;
            }
            ui.label("Output folder");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 80.0).max(120.0);
                    ui.add(egui::TextEdit::singleline(&mut self.st.out_dir).hint_text("(same as source folder)").desired_width(w));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.st.out_dir = s(&p);
                        }
                    }
                });
            });
            ui.end_row();
        });
        if refresh {
            self.refresh();
        }
        ui.horizontal(|ui| {
            if ui.button("Refresh list").clicked() {
                self.refresh();
            }
            if ui.button("Add files…").clicked() {
                if let Some(v) = rfd::FileDialog::new().add_filter("MKV", &["mkv"]).pick_files() {
                    self.add_files(v);
                }
            }
            if ui.button("All").clicked() {
                self.files.iter_mut().for_each(|f| f.1 = true);
            }
            if ui.button("None").clicked() {
                self.files.iter_mut().for_each(|f| f.1 = false);
            }
            ui.label(format!("{} of {} selected", self.files.iter().filter(|f| f.1).count(), self.files.len()));
        });
        if let Some(e) = &self.list_error {
            ui.colored_label(egui::Color32::LIGHT_RED, e);
        } else if self.files.is_empty() && !self.st.src_dir.trim().is_empty() {
            warn(ui, format!("No MKV files found in: {}", self.st.src_dir));
        }
        let dest_dir = self.dest_dir();
        egui::ScrollArea::vertical().max_height(260.0).id_salt("m9files").show(ui, |ui| {
            for (p, on) in &mut self.files {
                ui.horizontal(|ui| {
                    ui.checkbox(on, p.file_name().unwrap_or_default().to_string_lossy());
                    let dest = dest_dir.join(format!("{}.mp4", ffenc::util::stem(p)));
                    if dest.exists() {
                        hint(ui, "(MP4 exists – will be skipped)");
                    }
                });
            }
        });
        ui.add_space(6.0);
        ui.checkbox(&mut self.st.delete_sources, egui::RichText::new("Delete successfully converted source MKVs").color(egui::Color32::from_rgb(230, 120, 100)));
        hint(ui, "All tracks are copied. If a track is incompatible with MP4 that job fails and its MKV is kept.");
    }
    fn build(&mut self, _cx: &Ctx) -> Result<Plan, String> {
        let selected: Vec<PathBuf> = self.files.iter().filter(|f| f.1).map(|f| f.0.clone()).collect();
        if selected.is_empty() {
            return Err("Select at least one MKV file.".into());
        }
        let dest_dir = self.dest_dir();
        if dest_dir.as_os_str().is_empty() {
            return Err("Choose a source or output folder.".into());
        }
        std::fs::create_dir_all(&dest_dir).map_err(|e| format!("Cannot use output folder: {e}"))?;
        let plan = remux::plan(&selected, &dest_dir, &|p| p.exists(), ffenc::util::unique_id);
        let mut notes: Vec<String> = plan.skipped.iter().map(|(s, why)| format!("Skipping {}: {why}", s.display())).collect();
        if plan.jobs.is_empty() {
            return Err(format!("No files to process. Existing MP4s and source MKVs were kept.\n{}", notes.join("\n")));
        }
        if self.st.delete_sources {
            notes.push("Source MKVs will be DELETED after each successful conversion.".into());
        }
        let jobs = plan
            .jobs
            .iter()
            .map(|j| JobSpec {
                label: format!("Remux {} -> {}", j.source.file_name().unwrap_or_default().to_string_lossy(), j.dest.file_name().unwrap_or_default().to_string_lossy()),
                args: remux::args(j),
                output: Some(j.temp.clone()),
                log_file: Some(log_path_for(&j.dest)),
                duration_s: None,
                side_files: vec![],
                post: PostAction::Remux { temp: j.temp.clone(), dest: j.dest.clone(), delete_source: self.st.delete_sources.then(|| j.source.clone()) },
                warnings: vec![],
            })
            .collect();
        Ok(Plan { jobs, group: Some(("Remux".into(), true, plan.skipped.len())), notes })
    }
    fn on_drop(&mut self, paths: &[PathBuf], _cx: &Ctx) {
        if let Some(d) = paths.iter().find(|p| p.is_dir()) {
            self.st.src_dir = s(d);
            self.refresh();
        }
        self.add_files(paths.iter().filter(|p| p.is_file()).cloned());
    }
    fn save(&self) -> serde_json::Value {
        serde_json::to_value(&self.st).unwrap_or_default()
    }
    fn load(&mut self, v: &serde_json::Value, s: &Settings) {
        self.st = restore(v, State { src_dir: s.input_folder.clone(), ..Default::default() });
        if self.st.src_dir.trim().is_empty() {
            self.st.src_dir = s.input_folder.clone();
        }
        self.files.clear();
        self.listed_dir = None;
    }
}
