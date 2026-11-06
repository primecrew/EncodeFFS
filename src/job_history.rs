//! Job history: a persistent, human-readable log of every *successful* job – which preset it
//! came from, the output file, how long it took, the form settings and the exact FFmpeg command.
//! Failed and cancelled jobs are never written (they stay in the normal session Log).
//! The file is `job-history.log` in the logs folder; "Clear job history" only empties that file.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ffmpeg::runner::{Job, JobStatus};

pub const FILE_NAME: &str = "job-history.log";

/// What the form looked like when the jobs were queued.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub preset_name: String,
    /// Form values (`Mode::save()`); written as compact JSON.
    pub values: Value,
}

/// Remembers the origin of queued jobs and appends an entry when one succeeds.
#[derive(Default)]
pub struct Tracker {
    pending: HashMap<u64, Origin>,
}

/// One readable entry.
pub fn format_entry(when: &str, job: &Job, o: &Origin) -> String {
    let mut s = format!("=== {when} · {} · preset “{}”\n", job.spec.label, o.preset_name);
    if let Some(p) = job.final_output.as_ref().or(job.spec.output.as_ref()) {
        s += &format!("Output:   {}\n", p.display());
    }
    if let Some(d) = job.elapsed {
        s += &format!("Took:     {}\n", crate::finished::fmt_elapsed(d));
    }
    s += &format!("Settings: {}\n", serde_json::to_string(&o.values).unwrap_or_default());
    s += &format!("Command:  {}\n\n", crate::cmdline::cmd_line(&job.program, &job.spec.args));
    s
}

impl Tracker {
    pub fn register(&mut self, job_ids: &[u64], origin: Origin) {
        for &id in job_ids {
            self.pending.insert(id, origin.clone());
        }
    }

    /// Call with the current jobs. Returns the entries of jobs that just succeeded (in order);
    /// failed/cancelled jobs are dropped without an entry.
    pub fn update(&mut self, jobs: &[Job], now: &dyn Fn() -> String) -> Vec<String> {
        let mut out = vec![];
        for j in jobs.iter().filter(|j| j.status.finished()) {
            if let Some(o) = self.pending.remove(&j.id) {
                if j.status == JobStatus::Done {
                    out.push(format_entry(&now(), j, &o));
                }
            }
        }
        self.pending.retain(|id, _| jobs.iter().any(|j| j.id == *id));
        out
    }
}

pub fn path_in(logs_dir: &Path) -> PathBuf {
    logs_dir.join(FILE_NAME)
}

pub fn append(path: &Path, entries: &[String]) -> std::io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    for e in entries {
        f.write_all(e.as_bytes())?;
    }
    Ok(())
}

/// Text for the viewer ("" when there is no history yet).
pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Clear the history: removes only this file (nothing else in the folder).
pub fn clear(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::runner::{JobSpec, PostAction};
    use serde_json::json;

    fn job(id: u64, status: JobStatus) -> Job {
        Job {
            id,
            spec: JobSpec { label: format!("Re-encode {id}.mp4"), args: vec!["-i".into(), "in.mp4".into(), "out.mkv".into()], output: Some(PathBuf::from("out.mkv")), log_file: None, duration_s: None, side_files: vec![], keep_side_files: false, post: PostAction::None, warnings: vec![] },
            program: PathBuf::from("ffmpeg"),
            status,
            progress: None,
            pos_s: 0.0,
            speed: String::new(),
            cancel: false,
            group: None,
            keep_log: false,
            elapsed: Some(std::time::Duration::from_secs(5)),
            final_output: Some(PathBuf::from("out (2).mkv")),
        }
    }
    fn origin() -> Origin {
        Origin { preset_name: "Re-encode video".into(), values: json!({"video": {"crf": 20}}) }
    }
    const NOW: &dyn Fn() -> String = &|| "2026-11-06 14:00".to_string();

    #[test]
    fn only_successes_are_logged_once() {
        let mut t = Tracker::default();
        t.register(&[1, 2, 3, 4], origin());
        assert!(t.update(&[job(1, JobStatus::Running)], NOW).is_empty());
        let jobs = [job(1, JobStatus::Done), job(2, JobStatus::Failed("x".into())), job(3, JobStatus::Cancelled), job(4, JobStatus::Pending)];
        let e = t.update(&jobs, NOW);
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("Re-encode 1.mp4") && e[0].contains("preset “Re-encode video”"));
        assert!(e[0].contains("Output:   out (2).mkv"), "final name after keep-both");
        assert!(e[0].contains(r#"{"video":{"crf":20}}"#) && e[0].contains("out.mkv"));
        assert!(t.update(&jobs, NOW).is_empty(), "never twice");
        // unregistered jobs (e.g. started before) are ignored; cleared jobs are forgotten
        assert!(t.update(&[], NOW).is_empty());
        assert!(t.pending.is_empty());
    }

    #[test]
    fn persists_and_clears_only_its_file() {
        let dir = std::env::temp_dir().join(format!("ffenc_hist_{}", crate::util::unique_id()));
        let p = path_in(&dir);
        let other = dir.join("FFmpegEncoder.log");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&other, "x").unwrap();
        assert_eq!(read(&p), "");
        append(&p, &["a\n".into()]).unwrap();
        append(&p, &["b\n".into()]).unwrap();
        assert_eq!(read(&p), "a\nb\n");
        clear(&p).unwrap();
        clear(&p).unwrap();
        assert_eq!(read(&p), "");
        assert!(other.exists(), "other files untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
