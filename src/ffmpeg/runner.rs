//! Job queue: runs ffmpeg jobs one at a time on a worker thread, parses
//! `-progress pipe:1`, streams stderr to the live log + a per-job log file,
//! and supports clean cancellation ("q" on stdin, then kill after a grace period).

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::progress::{self, ProgressEvent};
use crate::cmd::remux::{publish_no_overwrite, RemuxSummary};

/// Global options the runner adds in front of every job's arguments.
pub const RUNNER_PREFIX: [&str; 4] = ["-hide_banner", "-nostats", "-progress", "pipe:1"];

pub fn full_args(args: &[String]) -> Vec<String> {
    RUNNER_PREFIX.iter().map(|s| s.to_string()).chain(args.iter().cloned()).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum PostAction {
    None,
    /// Mode 9: move the temp MP4 to its final name without overwriting; optionally delete the MKV.
    Remux { temp: PathBuf, dest: PathBuf, delete_source: Option<PathBuf> },
    /// Every other job (0.3.3): ffmpeg writes `temp` (same folder); after success it is renamed to
    /// `dest` – replacing an existing file only when `overwrite` was confirmed.
    Publish { temp: PathBuf, dest: PathBuf, overwrite: bool },
}

/// What a mode hands to the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct JobSpec {
    pub label: String,
    pub args: Vec<String>,
    /// File ffmpeg writes (checked for existence/non-empty after the run).
    pub output: Option<PathBuf>,
    /// stderr log (script: ffmpeg_error*.log in the output folder).
    pub log_file: Option<PathBuf>,
    /// Expected output duration for the progress bar.
    pub duration_s: Option<f64>,
    pub side_files: Vec<(PathBuf, String)>,
    /// Side files the user asked to keep (e.g. a saved .avs script): written, never deleted.
    pub keep_side_files: bool,
    pub post: PostAction,
    pub warnings: Vec<String>,
    /// GPU encode (1.1): the same command with the CPU encoder. When the GPU run fails, the
    /// runner warns and runs this instead, so one bad file doesn't fail a batch.
    pub cpu_fallback: Option<Vec<String>>,
}

impl JobSpec {
    pub fn new(label: impl Into<String>, cmd: crate::cmd::BuiltCommand, output: PathBuf, log_file: PathBuf, duration_s: Option<f64>) -> Self {
        JobSpec {
            label: label.into(),
            args: cmd.args,
            output: Some(output),
            log_file: Some(log_file),
            duration_s,
            side_files: cmd.side_files,
            keep_side_files: false,
            post: PostAction::None,
            warnings: cmd.warnings,
            cpu_fallback: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl JobStatus {
    pub fn finished(&self) -> bool {
        matches!(self, JobStatus::Done | JobStatus::Failed(_) | JobStatus::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub spec: JobSpec,
    pub program: PathBuf,
    pub status: JobStatus,
    pub progress: Option<f32>,
    pub pos_s: f64,
    pub speed: String,
    pub cancel: bool,
    pub group: Option<u64>,
    pub keep_log: bool,
    pub elapsed: Option<Duration>,
    /// The file a successful job left behind under its final name (after keep-both numbering).
    pub final_output: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct GroupState {
    pub label: String,
    pub remux: bool,
    pub summary: RemuxSummary,
    pub remaining: usize,
    pub done: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Default)]
pub struct Shared {
    pub jobs: Vec<Job>,
    pub log: VecDeque<String>,
    pub groups: HashMap<u64, GroupState>,
    next_id: u64,
    app_log: Option<File>,
    /// Keep chapter/list side files and partial outputs (debugging).
    pub keep_temp: bool,
    /// List of partial files of running jobs (cleaned at the next start after a crash).
    pub partials_registry: Option<PathBuf>,
}

const MAX_LOG_LINES: usize = 5000;

impl Shared {
    pub fn push_log(&mut self, line: impl Into<String>) {
        let line = line.into();
        if let Some(f) = self.app_log.as_mut() {
            let _ = writeln!(f, "{line}");
        }
        self.log.push_back(line);
        while self.log.len() > MAX_LOG_LINES {
            self.log.pop_front();
        }
    }
    fn job_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }
}

type Repaint = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct Queue {
    pub shared: Arc<Mutex<Shared>>,
    repaint: Repaint,
}

impl Queue {
    /// Create the queue and start its worker thread.
    pub fn new(app_log_path: Option<PathBuf>, repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let mut shared = Shared::default();
        if let Some(p) = app_log_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // simple rotation
            if std::fs::metadata(&p).map(|m| m.len() > 5_000_000).unwrap_or(false) {
                let _ = std::fs::rename(&p, p.with_extension("old.log"));
            }
            shared.app_log = std::fs::OpenOptions::new().create(true).append(true).open(&p).ok();
        }
        let q = Queue { shared: Arc::new(Mutex::new(shared)), repaint: Arc::new(repaint) };
        let qc = q.clone();
        thread::Builder::new().name("ffmpeg-worker".into()).spawn(move || qc.worker()).expect("spawn worker");
        q
    }

    /// "Keep temporary files for debugging".
    pub fn set_keep_temp(&self, on: bool) {
        self.shared.lock().unwrap().keep_temp = on;
    }

    pub fn set_partials_registry(&self, p: Option<PathBuf>) {
        self.shared.lock().unwrap().partials_registry = p;
    }

    fn keep_temp(&self) -> bool {
        self.shared.lock().unwrap().keep_temp
    }

    pub fn log(&self, line: impl Into<String>) {
        self.shared.lock().unwrap().push_log(line);
        (self.repaint)();
    }

    /// Add jobs. `group` creates a summary for a batch (remux=true for mode 9 wording).
    pub fn enqueue(&self, program: &Path, specs: Vec<JobSpec>, group: Option<(String, bool, usize)>, keep_log: bool) -> Vec<u64> {
        let mut s = self.shared.lock().unwrap();
        let gid = group.map(|(label, remux, preskipped)| {
            s.next_id += 1;
            let gid = s.next_id;
            let summary = RemuxSummary { skipped: preskipped, ..Default::default() };
            s.groups.insert(gid, GroupState { label, remux, summary, remaining: specs.len(), ..Default::default() });
            gid
        });
        let mut ids = vec![];
        for spec in specs {
            s.next_id += 1;
            let id = s.next_id;
            s.push_log(format!("Queued: {}", spec.label));
            s.jobs.push(Job {
                id,
                spec,
                program: program.to_path_buf(),
                status: JobStatus::Pending,
                progress: None,
                pos_s: 0.0,
                speed: String::new(),
                cancel: false,
                group: gid,
                keep_log,
                elapsed: None,
                final_output: None,
            });
            ids.push(id);
        }
        drop(s);
        (self.repaint)();
        ids
    }

    /// Cancel one job (pending -> cancelled immediately; running -> stop ffmpeg).
    pub fn cancel(&self, id: u64) {
        let mut s = self.shared.lock().unwrap();
        let mut finished_group = None;
        if let Some(j) = s.job_mut(id) {
            match j.status {
                JobStatus::Pending => {
                    j.status = JobStatus::Cancelled;
                    finished_group = j.group;
                }
                JobStatus::Running => j.cancel = true,
                _ => {}
            }
        }
        if let Some(g) = finished_group {
            Self::group_job_finished(&mut s, g, &JobStatus::Cancelled);
        }
        drop(s);
        (self.repaint)();
    }

    pub fn cancel_all(&self) {
        let ids: Vec<u64> = self.shared.lock().unwrap().jobs.iter().filter(|j| !j.status.finished()).map(|j| j.id).collect();
        for id in ids {
            self.cancel(id);
        }
    }

    pub fn clear_finished(&self) {
        self.shared.lock().unwrap().jobs.retain(|j| !j.status.finished());
        (self.repaint)();
    }

    pub fn is_busy(&self) -> bool {
        self.shared.lock().unwrap().jobs.iter().any(|j| !j.status.finished())
    }

    fn group_job_finished(s: &mut Shared, gid: u64, status: &JobStatus) {
        let Some(g) = s.groups.get_mut(&gid) else { return };
        g.remaining = g.remaining.saturating_sub(1);
        match status {
            JobStatus::Done => g.done += 1,
            JobStatus::Cancelled => g.cancelled += 1,
            _ => g.failed += 1,
        }
        if g.remaining == 0 {
            let line = if g.remux {
                let mut sum = g.summary.clone();
                sum.failed = g.failed + g.cancelled; // cancelled remuxes keep their MKV, like failures
                sum.text()
            } else {
                format!("{}: finished – {} done, {} failed, {} cancelled.", g.label, g.done, g.failed, g.cancelled)
            };
            s.push_log(line);
        }
    }

    fn worker(&self) {
        loop {
            let next = {
                let mut s = self.shared.lock().unwrap();
                let found = s.jobs.iter_mut().find(|j| j.status == JobStatus::Pending).map(|j| {
                    j.status = JobStatus::Running;
                    j.clone()
                });
                found
            };
            match next {
                Some(job) => {
                    let started = Instant::now();
                    let partial = match &job.spec.post {
                        PostAction::Publish { temp, .. } => Some(temp.clone()),
                        _ => None,
                    };
                    let registry = self.shared.lock().unwrap().partials_registry.clone();
                    if let (Some(r), Some(p)) = (&registry, &partial) {
                        crate::output_guard::register(r, p);
                    }
                    let mut status = self.run_job(&job);
                    if let (JobStatus::Failed(e), Some(cpu)) = (&status, job.spec.cpu_fallback.as_ref()) {
                        // warn, don't block: redo this file on the CPU encoder
                        self.log(format!("⚠ GPU encode failed for {} ({e}). Encoding this file again on the CPU (slower).", job.spec.label));
                        let mut again = job.clone();
                        again.spec.args = cpu.clone();
                        again.spec.cpu_fallback = None;
                        if let Some(p) = &partial {
                            // a kept (debugging) partial would stop the -n run
                            let _ = std::fs::remove_file(p);
                        }
                        self.set_progress(job.id, |j| {
                            j.progress = None;
                            j.pos_s = 0.0;
                            j.speed.clear();
                        });
                        status = self.run_job(&again);
                        if status == JobStatus::Done {
                            self.log(format!("Done on the CPU: {}", job.spec.label));
                        }
                    }
                    if let (Some(r), Some(p)) = (&registry, &partial) {
                        if !p.exists() || !self.keep_temp() {
                            crate::output_guard::unregister(r, p);
                        }
                    }
                    let mut s = self.shared.lock().unwrap();
                    let msg = match &status {
                        JobStatus::Done => format!("Processing complete: {}", job.spec.label),
                        JobStatus::Failed(e) => format!("FAILED: {} – {}", job.spec.label, e),
                        JobStatus::Cancelled => format!("Cancelled: {}", job.spec.label),
                        _ => String::new(),
                    };
                    s.push_log(msg);
                    if let Some(j) = s.job_mut(job.id) {
                        j.status = status.clone();
                        j.elapsed = Some(started.elapsed());
                        if status == JobStatus::Done {
                            j.progress = Some(1.0);
                            if j.final_output.is_none() {
                                j.final_output = j.spec.output.clone();
                            }
                        }
                    }
                    if let Some(g) = job.group {
                        Self::group_job_finished(&mut s, g, &status);
                    }
                    drop(s);
                    (self.repaint)();
                }
                None => thread::sleep(Duration::from_millis(150)),
            }
        }
    }

    fn set_progress(&self, id: u64, f: impl FnOnce(&mut Job)) {
        if let Some(j) = self.shared.lock().unwrap().job_mut(id) {
            f(j);
        }
        (self.repaint)();
    }

    fn cancel_requested(&self, id: u64) -> bool {
        self.shared.lock().unwrap().jobs.iter().any(|j| j.id == id && j.cancel)
    }

    /// Run one job to completion. Returns the final status.
    fn run_job(&self, job: &Job) -> JobStatus {
        let spec = &job.spec;
        let args = full_args(&spec.args);
        self.log(format!("Running: {}", crate::cmdline::cmd_line(&job.program, &args)));

        for (path, text) in &spec.side_files {
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(path, text) {
                return JobStatus::Failed(format!("Could not write {}: {e}", path.display()));
            }
        }
        if let Some(out) = spec.output.as_ref().and_then(|o| o.parent()) {
            if let Err(e) = std::fs::create_dir_all(out) {
                return JobStatus::Failed(format!("Cannot create output folder {}: {e}", out.display()));
            }
        }
        let mut log_file = spec.log_file.as_ref().and_then(|p| File::create(p).ok());
        if let Some(f) = log_file.as_mut() {
            let _ = writeln!(f, "{}\n", crate::cmdline::cmd_line(&job.program, &args));
        }

        let child = super::hidden_command(&job.program)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                self.cleanup_side_files(spec);
                return JobStatus::Failed(format!("Could not start ffmpeg ({}): {e}", job.program.display()));
            }
        };

        // progress reader
        let stdout = child.stdout.take().unwrap();
        let qp = self.clone();
        let id = job.id;
        let total = Arc::new(Mutex::new(spec.duration_s));
        let total_p = total.clone();
        let t_out = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match progress::parse_line(&line) {
                    Some(ProgressEvent::OutTime(t)) => {
                        let tot = *total_p.lock().unwrap();
                        qp.set_progress(id, |j| {
                            j.pos_s = t;
                            j.progress = progress::fraction(t, tot);
                        });
                    }
                    Some(ProgressEvent::Speed(sp)) => qp.set_progress(id, |j| j.speed = sp),
                    Some(ProgressEvent::End) => qp.set_progress(id, |j| j.progress = Some(1.0)),
                    None => {}
                }
            }
        });

        // stderr -> log file + live log
        let stderr = child.stderr.take().unwrap();
        let qe = self.clone();
        let total_e = total.clone();
        let t_err = thread::spawn(move || {
            let mut r = BufReader::new(stderr);
            let mut buf = Vec::new();
            let mut tail: VecDeque<String> = VecDeque::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let text = String::from_utf8_lossy(&buf);
                for line in text.split('\r').map(|l| l.trim_end()).filter(|l| !l.is_empty()) {
                    if let Some(f) = log_file.as_mut() {
                        let _ = writeln!(f, "{line}");
                    }
                    {
                        let mut t = total_e.lock().unwrap();
                        if t.is_none() {
                            *t = progress::parse_duration_line(line);
                        }
                    }
                    tail.push_back(line.to_string());
                    if tail.len() > 15 {
                        tail.pop_front();
                    }
                    qe.log(format!("  {line}"));
                }
            }
            tail
        });

        let mut stdin = child.stdin.take();
        let mut cancel_at: Option<Instant> = None;
        let exit = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) => {}
                Err(_) => break None,
            }
            if cancel_at.is_none() && self.cancel_requested(job.id) {
                // Ask ffmpeg to stop cleanly; it finalizes the file and exits.
                if let Some(mut si) = stdin.take() {
                    let _ = si.write_all(b"q");
                    let _ = si.flush();
                }
                cancel_at = Some(Instant::now());
            }
            if let Some(t) = cancel_at {
                if t.elapsed() > Duration::from_secs(4) {
                    let _ = child.kill();
                }
            }
            thread::sleep(Duration::from_millis(100));
        };
        drop(stdin);
        let _ = t_out.join();
        let tail = t_err.join().unwrap_or_default();
        self.cleanup_side_files(spec);

        if cancel_at.is_some() {
            // Remove the partial output (for remux this is the temp file).
            let partial = match &spec.post {
                PostAction::Remux { temp, .. } | PostAction::Publish { temp, .. } => Some(temp.clone()),
                PostAction::None => spec.output.clone(),
            };
            if let Some(p) = partial {
                if matches!(spec.post, PostAction::Publish { .. }) && self.keep_temp() {
                    if p.exists() {
                        self.log(format!("Kept partial output (debugging): {}", p.display()));
                    }
                } else {
                    let _ = std::fs::remove_file(p);
                }
            }
            return JobStatus::Cancelled;
        }

        let ok = exit.map(|s| s.success()).unwrap_or(false);
        let code = exit.and_then(|s| s.code()).map(|c| c.to_string()).unwrap_or_else(|| "?".into());
        let log_hint = spec.log_file.as_ref().map(|p| format!(" See log: {}", p.display())).unwrap_or_default();
        if !ok {
            if let PostAction::Remux { temp, .. } = &spec.post {
                let _ = std::fs::remove_file(temp);
            }
            if let PostAction::Publish { temp, .. } = &spec.post {
                if self.keep_temp() {
                    if temp.exists() {
                        self.log(format!("Kept partial output (debugging): {}", temp.display()));
                    }
                } else {
                    let _ = std::fs::remove_file(temp);
                }
            }
            let last = tail.back().cloned().unwrap_or_default();
            return JobStatus::Failed(format!("FFmpeg exited with code {code}. {last}{log_hint}"));
        }
        let check = match &spec.post {
            PostAction::Publish { temp, .. } => Some(temp.clone()),
            _ => spec.output.clone(),
        };
        if let Some(out) = &check {
            let good = std::fs::metadata(out).map(|m| m.len() > 0).unwrap_or(false);
            if !good {
                return JobStatus::Failed(format!("Output file not created: {}{log_hint}", out.display()));
            }
        }
        // Post actions
        if let PostAction::Publish { temp, dest, overwrite } = &spec.post {
            match crate::output_guard::publish(temp, dest, *overwrite) {
                Ok(used) => {
                    self.set_progress(job.id, |j| j.final_output = Some(used.clone()));
                    if &used != dest {
                        self.log(format!("{} appeared while encoding; the new file was saved as {} instead.", dest.display(), used.display()));
                    } else if *overwrite {
                        self.log(format!("Replaced {}", dest.display()));
                    }
                }
                Err(e) => {
                    return JobStatus::Failed(format!("Could not move the finished file {} to {}: {e}. The finished file was kept under its temporary name.{log_hint}", temp.display(), dest.display()));
                }
            }
        }
        if let PostAction::Remux { temp, dest, delete_source } = &spec.post {
            if let Err(e) = publish_no_overwrite(temp, dest) {
                let _ = std::fs::remove_file(temp);
                return JobStatus::Failed(format!("{e}. Source file kept."));
            }
            self.set_progress(job.id, |j| j.final_output = Some(dest.clone()));
            let mut s = self.shared.lock().unwrap();
            let mut deleted = None;
            if let Some(src) = delete_source {
                // Only after the output is verified non-empty at its final name.
                let good = std::fs::metadata(dest).map(|m| m.len() > 0).unwrap_or(false);
                deleted = Some(if good { std::fs::remove_file(src).map_err(|e| e.to_string()) } else { Err("Completed output is missing or empty; source file kept.".into()) });
            }
            if let Some(g) = job.group.and_then(|g| s.groups.get_mut(&g)) {
                g.summary.converted += 1;
                match &deleted {
                    Some(Ok(())) => g.summary.deleted += 1,
                    Some(Err(_)) => g.summary.delete_failures += 1,
                    None => {}
                }
            }
            match deleted {
                Some(Ok(())) => s.push_log(format!("Deleted source file: {}", delete_source.as_ref().unwrap().display())),
                Some(Err(e)) => s.push_log(format!("Could not delete {}: {e}", delete_source.as_ref().unwrap().display())),
                None => {}
            }
        }
        if !job.keep_log {
            if let Some(p) = &spec.log_file {
                let _ = std::fs::remove_file(p);
            }
        }
        JobStatus::Done
    }

    fn cleanup_side_files(&self, spec: &JobSpec) {
        if spec.keep_side_files {
            return;
        }
        if self.keep_temp() {
            for (p, _) in &spec.side_files {
                self.log(format!("Kept temporary file (debugging): {}", p.display()));
            }
            return;
        }
        for (p, _) in &spec.side_files {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GPU job that fails is encoded again with the CPU command (warn, don't block).
    /// Needs an ffmpeg on PATH; skipped otherwise.
    #[test]
    fn failed_gpu_job_is_redone_on_the_cpu() {
        let ok = std::process::Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false);
        if !ok {
            return;
        }
        let dir = std::env::temp_dir().join(format!("ffenc-gpu-fallback-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("out.mkv");
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let head = ["-y", "-f", "lavfi", "-i", "testsrc2=s=320x240:r=10:d=1", "-c:v"];
        // an invalid preset fails on every machine, with or without a GPU
        let gpu = [&head[..], &["h264_nvenc", "-preset", "no-such-preset", out.to_str().unwrap()]].concat();
        let cpu = [&head[..], &["libx264", "-preset", "ultrafast", out.to_str().unwrap()]].concat();
        let spec = JobSpec { label: "gpu job".into(), args: s(&gpu), output: Some(out.clone()), log_file: None, duration_s: Some(1.0), side_files: vec![], keep_side_files: false, post: PostAction::None, warnings: vec![], cpu_fallback: Some(s(&cpu)) };
        let q = Queue::new(None, || {});
        q.enqueue(Path::new("ffmpeg"), vec![spec], None, false);
        let t0 = Instant::now();
        while q.is_busy() {
            assert!(t0.elapsed() < Duration::from_secs(60), "timed out");
            thread::sleep(Duration::from_millis(50));
        }
        let s = q.shared.lock().unwrap();
        assert_eq!(s.jobs[0].status, JobStatus::Done);
        assert!(s.log.iter().any(|l| l.starts_with("⚠ GPU encode failed for gpu job")));
        assert!(out.exists());
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
