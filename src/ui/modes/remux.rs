//! Remux – stream-copy into a chosen container (never overwrite).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use super::{restore, Ctx, Mode, Plan};
use crate::ui::common::*;
use ffenc::cmd::remux::{self, Container};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::{JobSpec, PostAction};
use ffenc::settings::Settings;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct State {
    src_dir: String,
    /// Empty = same as the source folder (script default).
    out_dir: String,
    /// Default is MP4 so older “Remux MKV -> MP4” presets load as Remux with MP4.
    container: Container,
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
    probes: HashMap<PathBuf, Result<MediaInfo, String>>,
    scan: Option<Bg<Vec<(PathBuf, Result<MediaInfo, String>)>>>,
}

fn list_videos(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && remux::is_remux_input(p))
        .collect();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(v)
}

impl RemuxMode {
    fn refresh(&mut self) {
        self.listed_dir = Some(self.st.src_dir.clone());
        self.list_error = None;
        self.probes.clear();
        self.scan = None;
        match list_videos(Path::new(self.st.src_dir.trim())) {
            Ok(v) => self.files = v.into_iter().map(|p| (p, true)).collect(),
            Err(e) => {
                self.files.clear();
                self.list_error = Some(e);
            }
        }
    }
    fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            if remux::is_remux_input(&p) && !self.files.iter().any(|(f, _)| *f == p) {
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

    fn dest_of(&self, src: &Path) -> PathBuf {
        self.dest_dir().join(format!("{}.{}", ffenc::util::stem(src), self.st.container.ext()))
    }

    fn poll_probes(&mut self, cx: &Ctx) {
        if let Some(t) = &self.scan {
            if let Some(list) = t.poll() {
                self.scan = None;
                for (p, r) in list {
                    self.probes.insert(p, r);
                }
            }
        }
        self.start_probes(cx);
    }

    fn start_probes(&mut self, cx: &Ctx) {
        if self.scan.is_some() {
            return;
        }
        let Some(fp) = cx.ffprobe.map(|p| p.to_path_buf()) else {
            return;
        };
        let pending: Vec<PathBuf> = self.files.iter().map(|(p, _)| p.clone()).filter(|p| !self.probes.contains_key(p)).collect();
        if pending.is_empty() {
            return;
        }
        self.scan = Some(Bg::spawn(cx.egui, move || pending.into_iter().map(|p| {
            let r = probe::probe(&fp, &p);
            (p, r)
        }).collect()));
    }

    fn warnings_for(&self, path: &Path) -> Vec<String> {
        let Some(Ok(info)) = self.probes.get(path) else {
            return vec![];
        };
        Self::warnings_for_info(path, info, self.st.container)
    }

    fn warnings_for_info(path: &Path, info: &MediaInfo, container: Container) -> Vec<String> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        remux::copy_warnings_for(info, container).into_iter().map(|w| format!("{name}: {w}")).collect()
    }

    fn will_skip(&self, path: &Path) -> Option<String> {
        let ext = self.st.container.ext();
        if ffenc::util::ext_lower(path) == ext {
            return Some(format!("(already {} – will be skipped)", self.st.container.a_name()));
        }
        if self.dest_of(path).exists() {
            return Some(format!("({} exists – will be skipped)", self.st.container.label()));
        }
        None
    }
}

impl Mode for RemuxMode {
    fn key(&self) -> &'static str {
        "remux"
    }
    fn title(&self) -> &'static str {
        "Remux"
    }
    fn description(&self) -> &'static str {
        "Container copy (-map 0 -c copy). Never overwrites; files already in the chosen container are skipped."
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
        self.poll_probes(cx);
        let mut refresh = false;
        egui::Grid::new("m9").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            if path_row(ui, "Source folder", &mut self.st.src_dir, Pick::Folder, &cx.settings.input_folder) && PathBuf::from(self.st.src_dir.trim()).is_dir() {
                refresh = true;
            }
            crate::ui::common::row_label(ui, "Output folder");
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
            crate::ui::common::row_label(ui, "Output container");
            ui.horizontal(|ui| {
                for c in Container::ALL {
                    ui.radio_value(&mut self.st.container, c, c.label());
                }
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
                let (label, exts) = remux::INPUT_FILTER;
                if let Some(v) = rfd::FileDialog::new().add_filter(label, exts).pick_files() {
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
            ui.colored_label(crate::ui::theme::c().err, e);
        } else if self.files.is_empty() && !self.st.src_dir.trim().is_empty() {
            warn(ui, format!("No video files found in: {}", self.st.src_dir));
        }
        let skip_hints: Vec<Option<String>> = self.files.iter().map(|(p, _)| self.will_skip(p)).collect();
        egui::ScrollArea::vertical().max_height(260.0).id_salt("m9files").show(ui, |ui| {
            for ((p, on), skip) in self.files.iter_mut().zip(skip_hints.into_iter()) {
                ui.horizontal(|ui| {
                    ui.checkbox(on, p.file_name().unwrap_or_default().to_string_lossy());
                    if let Some(h) = skip {
                        hint(ui, h);
                    }
                });
            }
        });
        ui.add_space(6.0);
        ui.checkbox(&mut self.st.delete_sources, egui::RichText::new("Delete successfully remuxed source files").color(crate::ui::theme::c().err));
        if self.st.container == Container::Ts {
            hint(ui, remux::TS_CHAPTERS_NOTE);
        }
        for (p, on) in &self.files {
            if !*on {
                continue;
            }
            if self.will_skip(p).is_some() {
                continue;
            }
            for w in self.warnings_for(p) {
                warn(ui, w);
            }
        }
        if self.scan.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Checking tracks for {} compatibility…", self.st.container.label()));
            });
        }
        hint(ui, "All tracks are copied as they are. If a job fails, its source file is kept.");
    }
    fn build(&mut self, cx: &Ctx) -> Result<Plan, String> {
        let selected: Vec<PathBuf> = self.files.iter().filter(|f| f.1).map(|f| f.0.clone()).collect();
        if selected.is_empty() {
            return Err("Select at least one video file.".into());
        }
        let dest_dir = self.dest_dir();
        if dest_dir.as_os_str().is_empty() {
            return Err("Choose a source or output folder.".into());
        }
        std::fs::create_dir_all(&dest_dir).map_err(|e| format!("Cannot use output folder: {e}"))?;
        let plan = remux::plan(&selected, &dest_dir, self.st.container, &|p| p.exists(), ffenc::util::unique_id);
        let mut notes: Vec<String> = plan.skipped.iter().map(|(s, why)| format!("Skipping {}: {why}", s.display())).collect();
        if plan.jobs.is_empty() {
            return Err(format!(
                "No files to process. Files already in the chosen container and existing outputs were kept.\n{}",
                notes.join("\n")
            ));
        }
        if self.st.delete_sources {
            notes.push("Source files will be DELETED after each successful remux.".into());
        }
        if self.st.container == Container::Ts {
            notes.push(remux::TS_CHAPTERS_NOTE.into());
        }
        let container = self.st.container;
        let jobs = plan
            .jobs
            .iter()
            .map(|j| {
                let info = self.probes.get(&j.source).and_then(|r| r.as_ref().ok()).cloned().or_else(|| {
                    cx.ffprobe.and_then(|fp| probe::probe(fp, &j.source).ok())
                });
                let warnings = info.map(|i| Self::warnings_for_info(&j.source, &i, container)).unwrap_or_default();
                JobSpec {
                    label: format!("Remux {} -> {}", j.source.file_name().unwrap_or_default().to_string_lossy(), j.dest.file_name().unwrap_or_default().to_string_lossy()),
                    args: remux::args(j),
                    output: Some(j.temp.clone()),
                    log_file: Some(log_path_for(&j.dest)),
                    duration_s: None,
                    side_files: vec![],
                    keep_side_files: false,
                    post: PostAction::Remux { temp: j.temp.clone(), dest: j.dest.clone(), delete_source: self.st.delete_sources.then(|| j.source.clone()) },
                    warnings,
                }
            })
            .collect();
        Ok(Plan { jobs, group: Some(("Remux".into(), true, plan.skipped.len())), notes })
    }
    fn wants_folders(&self) -> bool {
        true
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
        // Older presets have no `container` field; serde default is MP4.
        self.files.clear();
        self.listed_dir = None;
        self.probes.clear();
        self.scan = None;
    }
}
