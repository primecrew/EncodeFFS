//! "Encoding Finished" popup: which successful outputs to announce, and how to show one in Explorer.
//! Pure logic (unit-tested); the window itself is in the UI crate.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ffmpeg::runner::{Job, JobStatus};

/// One successful output.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// Final file name/path (after keep-both numbering, never the temporary partial name).
    pub path: PathBuf,
    pub size: Option<u64>,
    pub elapsed: Option<Duration>,
}

/// What the popup shows: the successful outputs of a run, and how many other jobs of that run
/// failed or were cancelled (those are reported in the Log as before).
#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub items: Vec<Finished>,
    pub not_done: usize,
}

/// Collects finished jobs while the queue runs; when the queue is idle again and at least one job
/// succeeded, hands out one popup for the whole run (a single job, a batch, or several queued jobs).
#[derive(Default)]
pub struct Tracker {
    seen: HashSet<u64>,
    items: Vec<Finished>,
    not_done: usize,
}

impl Tracker {
    /// Call with the current jobs (e.g. every frame). `size_of` reads a file's size.
    pub fn update(&mut self, jobs: &[Job], size_of: &dyn Fn(&Path) -> Option<u64>) -> Option<Popup> {
        for j in jobs {
            if !j.status.finished() || !self.seen.insert(j.id) {
                continue;
            }
            match (&j.status, &j.final_output) {
                (JobStatus::Done, Some(p)) => self.items.push(Finished { path: p.clone(), size: size_of(p), elapsed: j.elapsed }),
                (JobStatus::Done, None) => {} // nothing written to show
                _ => self.not_done += 1,
            }
        }
        if jobs.iter().any(|j| !j.status.finished()) {
            return None;
        }
        let not_done = std::mem::take(&mut self.not_done);
        if self.items.is_empty() {
            return None; // only failures / cancellations: no popup
        }
        Some(Popup { items: std::mem::take(&mut self.items), not_done })
    }
}

/// Explorer's argument to open a folder with `path` selected: `/select,"<path>"`.
/// Explorer parses its own command line, so this must be passed verbatim (raw), not re-quoted.
/// `None` when the path can't be passed safely (a `"` or control character – neither is valid in a
/// Windows file name) or isn't absolute; callers then open the folder instead.
pub fn explorer_select_arg(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let absolute = s.starts_with("\\\\") || (s.len() >= 3 && s.as_bytes()[1] == b':' && matches!(s.as_bytes()[2], b'\\' | b'/'));
    if !absolute || s.chars().any(|c| c == '"' || c.is_control()) {
        return None;
    }
    Some(format!("/select,\"{}\"", s.replace('/', "\\")))
}

/// What "Go to file" does: select the file in Explorer, or open its folder when the file is gone
/// (or its path can't be passed safely).
#[derive(Debug, Clone, PartialEq)]
pub enum GoTo {
    Select(String),
    OpenFolder(PathBuf),
    Nothing,
}

pub fn go_to(path: &Path, exists: bool) -> GoTo {
    if exists {
        if let Some(a) = explorer_select_arg(path) {
            return GoTo::Select(a);
        }
    }
    // split at the last \ or / ourselves, so Windows paths behave the same in tests on any OS
    let s = path.to_string_lossy();
    match s.rfind(['\\', '/']) {
        Some(i) if i > 0 => GoTo::OpenFolder(PathBuf::from(&s[..i])),
        _ => GoTo::Nothing,
    }
}

/// "12.3 MB"
pub fn fmt_size(bytes: u64) -> String {
    const U: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// "1:02:03" / "4:05" / "12 s"
pub fn fmt_elapsed(d: Duration) -> String {
    let t = d.as_secs();
    match (t / 3600, (t / 60) % 60, t % 60) {
        (0, 0, s) => format!("{s} s"),
        (0, m, s) => format!("{m}:{s:02}"),
        (h, m, s) => format!("{h}:{m:02}:{s:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::runner::JobSpec;

    fn job(id: u64, status: JobStatus, out: Option<&str>) -> Job {
        Job {
            id,
            spec: JobSpec { label: format!("j{id}"), args: vec![], output: None, log_file: None, duration_s: None, side_files: vec![], keep_side_files: false, post: crate::ffmpeg::runner::PostAction::None, warnings: vec![], cpu_fallback: None },
            program: PathBuf::new(),
            status,
            progress: None,
            pos_s: 0.0,
            speed: String::new(),
            cancel: false,
            group: None,
            keep_log: false,
            elapsed: Some(Duration::from_secs(65)),
            final_output: out.map(PathBuf::from),
        }
    }
    fn sz(_: &Path) -> Option<u64> {
        Some(2048)
    }

    #[test]
    fn single_success_shows_once() {
        let mut t = Tracker::default();
        assert_eq!(t.update(&[job(1, JobStatus::Running, None)], &sz), None);
        let p = t.update(&[job(1, JobStatus::Done, Some(r"C:\out\a (2).mp4"))], &sz).unwrap();
        assert_eq!(p.items, vec![Finished { path: PathBuf::from(r"C:\out\a (2).mp4"), size: Some(2048), elapsed: Some(Duration::from_secs(65)) }]);
        assert_eq!(p.not_done, 0);
        assert_eq!(t.update(&[job(1, JobStatus::Done, Some(r"C:\out\a (2).mp4"))], &sz), None, "never twice");
    }

    #[test]
    fn batch_waits_for_the_whole_queue_and_counts_failures() {
        let mut t = Tracker::default();
        let mut jobs = vec![job(1, JobStatus::Done, Some("/o/1.mp4")), job(2, JobStatus::Running, None), job(3, JobStatus::Pending, None)];
        assert_eq!(t.update(&jobs, &sz), None, "queue still busy");
        jobs[1] = job(2, JobStatus::Failed("x".into()), None);
        jobs[2] = job(3, JobStatus::Done, Some("/o/3.mp4"));
        let p = t.update(&jobs, &sz).unwrap();
        assert_eq!(p.items.iter().map(|f| f.path.to_str().unwrap()).collect::<Vec<_>>(), ["/o/1.mp4", "/o/3.mp4"]);
        assert_eq!(p.not_done, 1);
    }

    #[test]
    fn no_popup_for_failed_or_cancelled_only() {
        let mut t = Tracker::default();
        assert_eq!(t.update(&[job(1, JobStatus::Failed("x".into()), None), job(2, JobStatus::Cancelled, None)], &sz), None);
        // a later success doesn't carry the old failures along
        let p = t.update(&[job(3, JobStatus::Done, Some("/o/3.mp4"))], &sz).unwrap();
        assert_eq!(p.not_done, 0);
        // cleared queue (jobs removed) is fine
        assert_eq!(t.update(&[], &sz), None);
    }

    #[test]
    fn explorer_select_quoting() {
        assert_eq!(explorer_select_arg(Path::new(r"C:\My Videos\Ünïcode ✓ (2).mp4")).unwrap(), r#"/select,"C:\My Videos\Ünïcode ✓ (2).mp4""#);
        assert_eq!(explorer_select_arg(Path::new(r"\\nas\share\a b.mkv")).unwrap(), r#"/select,"\\nas\share\a b.mkv""#);
        assert_eq!(explorer_select_arg(Path::new("C:/x/y.mp4")).unwrap(), r#"/select,"C:\x\y.mp4""#);
        // injection attempts / unsafe input are refused (no way to break out of the quotes)
        assert_eq!(explorer_select_arg(Path::new(r#"C:\a" & calc & ".mp4"#)), None);
        assert_eq!(explorer_select_arg(Path::new("C:\\a\nb.mp4")), None);
        assert_eq!(explorer_select_arg(Path::new("relative.mp4")), None);
    }

    #[test]
    fn go_to_falls_back_to_the_folder() {
        assert_eq!(go_to(Path::new(r"C:\o\a.mp4"), true), GoTo::Select(r#"/select,"C:\o\a.mp4""#.into()));
        assert_eq!(go_to(Path::new(r"C:\o\a.mp4"), false), GoTo::OpenFolder(PathBuf::from(r"C:\o")));
        assert_eq!(go_to(Path::new(r#"C:\o\a".mp4"#), true), GoTo::OpenFolder(PathBuf::from(r"C:\o")));
        assert_eq!(go_to(Path::new("a.mp4"), false), GoTo::Nothing);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_size(512), "512 bytes");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(fmt_elapsed(Duration::from_secs(12)), "12 s");
        assert_eq!(fmt_elapsed(Duration::from_secs(245)), "4:05");
        assert_eq!(fmt_elapsed(Duration::from_secs(3723)), "1:02:03");
    }
}
