//! Mode 9: MKV -> MP4 container copy, never overwriting anything.

use std::io;
use std::path::{Path, PathBuf};

use super::p;
use crate::util::stem;

#[derive(Debug, Clone, PartialEq)]
pub struct RemuxJob {
    pub source: PathBuf,
    pub dest: PathBuf,
    /// ffmpeg writes here first; renamed to `dest` only after success.
    pub temp: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemuxPlan {
    pub jobs: Vec<RemuxJob>,
    /// (source, reason)
    pub skipped: Vec<(PathBuf, String)>,
}

/// Plan the queue: one job per selected MKV, skipping existing outputs and duplicates.
/// `exists` is injected so the logic is testable without touching the disk.
pub fn plan(
    sources: &[PathBuf],
    dest_dir: &Path,
    exists: &dyn Fn(&Path) -> bool,
    mut temp_id: impl FnMut() -> String,
) -> RemuxPlan {
    let mut plan = RemuxPlan::default();
    for src in sources {
        if crate::util::ext_lower(src) != "mkv" {
            plan.skipped.push((src.clone(), "not an .mkv file".into()));
            continue;
        }
        let dest = dest_dir.join(format!("{}.mp4", stem(src)));
        if exists(&dest) {
            plan.skipped.push((src.clone(), format!("output already exists: {}", dest.display())));
            continue;
        }
        if plan.jobs.iter().any(|j| j.dest == dest) {
            plan.skipped.push((src.clone(), format!("another selected file already maps to {}", dest.display())));
            continue;
        }
        let temp = dest_dir.join(format!(".remux_{}.mp4", temp_id()));
        plan.jobs.push(RemuxJob { source: src.clone(), dest, temp });
    }
    plan
}

/// `-nostdin -n -i SRC -map 0 -c copy TEMP` (`-n` = never overwrite).
pub fn args(job: &RemuxJob) -> Vec<String> {
    vec![
        "-nostdin".into(), "-n".into(), "-i".into(), p(&job.source), "-map".into(), "0".into(), "-c".into(), "copy".into(),
        p(&job.temp),
    ]
}

/// Move `temp` to `dest` without ever replacing an existing `dest`.
///
/// `std::fs::rename` silently replaces on Windows, so we first try a hard link
/// (fails atomically if `dest` exists) and remove the temp; if the file system
/// cannot hard-link (FAT/exFAT, some network shares) we fall back to
/// check-then-rename.
pub fn publish_no_overwrite(temp: &Path, dest: &Path) -> io::Result<()> {
    let meta = std::fs::metadata(temp)?;
    if meta.len() == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "FFmpeg did not create a nonempty MP4"));
    }
    match std::fs::hard_link(temp, dest) {
        Ok(()) => {
            std::fs::remove_file(temp)?;
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("output appeared while processing: {}", dest.display()),
        )),
        Err(_) => {
            if dest.exists() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("output exists: {}", dest.display())));
            }
            std::fs::rename(temp, dest)
        }
    }
}

/// Queue summary in the script's wording.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemuxSummary {
    pub converted: usize,
    pub failed: usize,
    pub skipped: usize,
    pub deleted: usize,
    pub delete_failures: usize,
}
impl RemuxSummary {
    pub fn text(&self) -> String {
        format!(
            "Queue finished: {} converted, {} failed, {} skipped, {} source MKV(s) deleted, {} deletion failure(s).",
            self.converted, self.failed, self.skipped, self.deleted, self.delete_failures
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn plan_skips_existing_and_non_mkv() {
        let existing: HashSet<PathBuf> = [PathBuf::from("/out/b.mp4")].into_iter().collect();
        let mut n = 0;
        let plan = plan(
            &["/in/a.mkv".into(), "/in/b.mkv".into(), "/in/c.avi".into(), "/in2/a.mkv".into()],
            Path::new("/out"),
            &|p| existing.contains(p),
            || {
                n += 1;
                n.to_string()
            },
        );
        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].dest, PathBuf::from("/out/a.mp4"));
        assert_eq!(plan.jobs[0].temp, PathBuf::from("/out/.remux_1.mp4"));
        assert_eq!(plan.skipped.len(), 3);
        assert_eq!(
            args(&plan.jobs[0]).join(" "),
            "-nostdin -n -i /in/a.mkv -map 0 -c copy /out/.remux_1.mp4"
        );
    }

    #[test]
    fn publish_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("remux_test_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let temp = dir.join(".remux_x.mp4");
        let dest = dir.join("a.mp4");
        std::fs::write(&temp, b"data").unwrap();
        publish_no_overwrite(&temp, &dest).unwrap();
        assert!(!temp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
        // second publish must fail and keep the original
        std::fs::write(&temp, b"new").unwrap();
        assert!(publish_no_overwrite(&temp, &dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
        // empty temp is rejected
        std::fs::write(&temp, b"").unwrap();
        assert!(publish_no_overwrite(&temp, &dir.join("b.mp4")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn summary_text() {
        let s = RemuxSummary { converted: 2, failed: 1, skipped: 3, deleted: 2, delete_failures: 0 };
        assert_eq!(s.text(), "Queue finished: 2 converted, 1 failed, 3 skipped, 2 source MKV(s) deleted, 0 deletion failure(s).");
    }
}
