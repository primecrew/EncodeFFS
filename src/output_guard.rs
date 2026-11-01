//! Protection against overwriting files.
//!
//! * [`find_conflicts`] – outputs that already exist, equal one of the job's own inputs, or are
//!   claimed by an earlier job of the same queue (case-insensitive on Windows).
//! * [`resolve`] – turns the user's per-file [`Choice`] into the final output of every job
//!   (Overwrite / Keep both = Windows-style "name (2).ext" / Skip).
//! * [`prepare`] – rewrites a job so ffmpeg writes to a temporary "partial" file in the same
//!   folder (`-n`, or `-y` only for a confirmed overwrite); the runner renames it to the final
//!   name after success ([`publish`]), so a failed or cancelled encode never touches the
//!   existing file.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::runner::{JobSpec, PostAction};

/// Setting: what to do when an output file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WhenExists {
    /// Show the conflicts in the run confirmation and let the user choose (default).
    #[default]
    Ask,
    /// Always keep both: the new file gets the next free "name (2).ext".
    KeepBoth,
    /// Always replace the existing file.
    Overwrite,
    /// Always skip jobs whose output exists.
    Skip,
}

impl WhenExists {
    pub const ALL: [WhenExists; 4] = [WhenExists::Ask, WhenExists::KeepBoth, WhenExists::Overwrite, WhenExists::Skip];
    pub fn label(self) -> &'static str {
        match self {
            WhenExists::Ask => "Ask",
            WhenExists::KeepBoth => "Always keep both (auto-number)",
            WhenExists::Overwrite => "Always overwrite",
            WhenExists::Skip => "Always skip",
        }
    }
    /// The choice pre-selected for every conflict.
    pub fn initial_choice(self) -> Choice {
        match self {
            WhenExists::Ask | WhenExists::KeepBoth => Choice::KeepBoth,
            WhenExists::Overwrite => Choice::Overwrite,
            WhenExists::Skip => Choice::Skip,
        }
    }
}

/// Per-file decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Overwrite,
    KeepBoth,
    Skip,
}

impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Choice::Overwrite => "Overwrite",
            Choice::KeepBoth => "Keep both",
            Choice::Skip => "Skip",
        }
    }
}

/// Windows file systems compare names case-insensitively.
pub const CASE_INSENSITIVE: bool = cfg!(windows);

/// Comparison key for a path.
pub fn path_key(p: &Path, ci: bool) -> String {
    let mut s = p.to_string_lossy().into_owned();
    if ci {
        s = s.replace('/', "\\").to_lowercase();
        while s.len() > 3 && s.ends_with('\\') {
            s.pop();
        }
    }
    s
}

/// ("my.video", ".mp4"); no extension -> ("name", ""); ".hidden" -> (".hidden", "").
pub fn split_name(file_name: &str) -> (&str, &str) {
    match file_name.rfind('.') {
        Some(i) if i > 0 && i + 1 < file_name.len() => (&file_name[..i], &file_name[i..]),
        _ => (file_name, ""),
    }
}

/// "clip (3)" -> ("clip", Some(3)).
fn strip_number(stem: &str) -> (&str, Option<u32>) {
    if let Some(body) = stem.strip_suffix(')') {
        if let Some(i) = body.rfind(" (") {
            let num = &body[i + 2..];
            if !num.is_empty() && num.len() < 9 && num.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(n) = num.parse::<u32>() {
                    if n >= 1 && i > 0 {
                        return (&stem[..i], Some(n));
                    }
                }
            }
        }
    }
    (stem, None)
}

/// "dir/name (n).ext"
pub fn numbered(path: &Path, base: &str, ext: &str, n: u32) -> PathBuf {
    path.with_file_name(format!("{base} ({n}){ext}"))
}

/// Windows "keep both" naming: "name.ext" -> "name (2).ext", "name (3).ext", … the first one for
/// which `taken` is false. A name that already ends in " (N)" continues at N+1.
pub fn keep_both_name(path: &Path, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = split_name(&file);
    let (base, n0) = strip_number(stem);
    let start = n0.map(|n| n + 1).unwrap_or(2).max(2);
    for n in start..start + 100_000 {
        let c = numbered(path, base, ext, n);
        if !taken(&c) {
            return c;
        }
    }
    path.with_file_name(format!("{base} ({}){ext}", crate::util::unique_id()))
}

/// Size and date of an existing output.
#[derive(Debug, Clone, PartialEq)]
pub struct Existing {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

pub fn stat(p: &Path) -> Option<Existing> {
    let m = std::fs::metadata(p).ok()?;
    Some(Existing { size: m.len(), modified: m.modified().ok() })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    /// A file with that name is already there.
    Exists(Existing),
    /// The output is one of the job's own inputs (never overwritten).
    SameAsInput,
    /// An earlier job in the same run writes the same file (index of that job).
    SameAsJob(usize),
}

/// One job's output plus the files it reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub output: PathBuf,
    pub inputs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub job: usize,
    pub output: PathBuf,
    pub reasons: Vec<Reason>,
}

impl Conflict {
    /// Overwrite is only possible for a plain existing file (not an input, not another job's output).
    pub fn can_overwrite(&self) -> bool {
        self.reasons.iter().all(|r| matches!(r, Reason::Exists(_)))
    }
    pub fn existing(&self) -> Option<&Existing> {
        self.reasons.iter().find_map(|r| if let Reason::Exists(e) = r { Some(e) } else { None })
    }
}

/// Every job whose output needs a decision.
pub fn find_conflicts(targets: &[Target], stat: &dyn Fn(&Path) -> Option<Existing>, ci: bool) -> Vec<Conflict> {
    let mut out = vec![];
    for (i, t) in targets.iter().enumerate() {
        let k = path_key(&t.output, ci);
        let mut reasons = vec![];
        if t.inputs.iter().any(|inp| path_key(inp, ci) == k) {
            reasons.push(Reason::SameAsInput);
        }
        if let Some(e) = stat(&t.output) {
            reasons.push(Reason::Exists(e));
        }
        if let Some(j) = targets[..i].iter().position(|o| path_key(&o.output, ci) == k) {
            reasons.push(Reason::SameAsJob(j));
        }
        if !reasons.is_empty() {
            out.push(Conflict { job: i, output: t.output.clone(), reasons });
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Run { output: PathBuf, overwrite: bool },
    Skip,
}

impl Outcome {
    pub fn output(&self) -> Option<&Path> {
        match self {
            Outcome::Run { output, .. } => Some(output),
            Outcome::Skip => None,
        }
    }
}

/// Decide each job's final output. `choice(i)` is only asked for jobs that conflict.
/// Overwrite falls back to Keep both when it is not allowed (output = an input, or the same
/// output as an earlier job of this run). Renamed outputs avoid files on disk, every output
/// of this run and every input.
pub fn resolve(targets: &[Target], choice: &dyn Fn(usize) -> Choice, exists: &dyn Fn(&Path) -> bool, ci: bool) -> Vec<Outcome> {
    let planned: HashSet<String> = targets.iter().map(|t| path_key(&t.output, ci)).collect();
    let inputs: HashSet<String> = targets.iter().flat_map(|t| t.inputs.iter().map(|p| path_key(p, ci))).collect();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut out = vec![];
    for (i, t) in targets.iter().enumerate() {
        let k = path_key(&t.output, ci);
        let own_input = t.inputs.iter().any(|p| path_key(p, ci) == k);
        let on_disk = exists(&t.output);
        let dup = claimed.contains(&k);
        let o = if !own_input && !on_disk && !dup {
            Outcome::Run { output: t.output.clone(), overwrite: false }
        } else {
            match choice(i) {
                Choice::Skip => Outcome::Skip,
                Choice::Overwrite if !own_input && !dup => Outcome::Run { output: t.output.clone(), overwrite: on_disk },
                _ => {
                    let taken = |p: &Path| {
                        let pk = path_key(p, ci);
                        exists(p) || claimed.contains(&pk) || planned.contains(&pk) || inputs.contains(&pk)
                    };
                    Outcome::Run { output: keep_both_name(&t.output, &taken), overwrite: false }
                }
            }
        };
        if let Some(p) = o.output() {
            claimed.insert(path_key(p, ci));
        }
        out.push(o);
    }
    out
}

/// Files ffmpeg reads in this job: every `-i` value except the app's own temp side files.
pub fn job_inputs(spec: &JobSpec) -> Vec<PathBuf> {
    let side: Vec<&PathBuf> = spec.side_files.iter().map(|(p, _)| p).collect();
    spec.args
        .windows(2)
        .filter(|w| w[0] == "-i")
        .map(|w| PathBuf::from(&w[1]))
        .filter(|p| !side.contains(&p))
        .collect()
}

/// Marker in temporary output names.
pub const PARTIAL_TAG: &str = ".ffenc-partial-";

/// "dir/name.ffenc-partial-<id>.ext" – same folder (so the final rename is atomic) and same
/// extension (so ffmpeg picks the right muxer).
pub fn partial_path(final_out: &Path, id: &str) -> PathBuf {
    let file = final_out.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = split_name(&file);
    final_out.with_file_name(format!("{stem}{PARTIAL_TAG}{id}{ext}"))
}

pub fn is_partial(p: &Path) -> bool {
    p.file_name().map(|f| f.to_string_lossy().contains(PARTIAL_TAG)).unwrap_or(false)
}

/// Rewrite a job to write `final_out` via a partial file. `-y` only when `overwrite` was
/// confirmed, otherwise `-n` (ffmpeg refuses to replace anything).
pub fn prepare(mut spec: JobSpec, final_out: &Path, overwrite: bool, id: &str) -> Result<JobSpec, String> {
    let orig = spec.output.clone().ok_or("job has no output")?;
    let orig_s = orig.to_string_lossy().into_owned();
    let pos = spec.args.iter().rposition(|a| *a == orig_s).ok_or_else(|| format!("output {} not found in the command", orig.display()))?;
    let temp = partial_path(final_out, id);
    spec.args[pos] = temp.to_string_lossy().into_owned();
    spec.args.retain(|a| a != "-y" && a != "-n");
    spec.args.insert(0, if overwrite { "-y".into() } else { "-n".into() });
    if final_out != orig {
        let name = final_out.file_name().unwrap_or_default().to_string_lossy();
        spec.label = format!("{} (saved as “{name}”)", spec.label);
    }
    spec.output = Some(final_out.to_path_buf());
    spec.post = PostAction::Publish { temp, dest: final_out.to_path_buf(), overwrite };
    Ok(spec)
}

/// Move a finished partial to its final name. Without `overwrite` an existing file is never
/// replaced: if one appeared meanwhile, the result goes to the next free "name (n).ext".
/// Returns the name actually used.
pub fn publish(temp: &Path, dest: &Path, overwrite: bool) -> io::Result<PathBuf> {
    let len = std::fs::metadata(temp)?.len();
    if len == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "FFmpeg produced an empty file"));
    }
    if overwrite {
        // MoveFileEx(REPLACE_EXISTING) on Windows: the old file is replaced in one step.
        std::fs::rename(temp, dest)?;
        return Ok(dest.to_path_buf());
    }
    match crate::cmd::remux::publish_no_overwrite(temp, dest) {
        Ok(()) => Ok(dest.to_path_buf()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let alt = keep_both_name(dest, &|p| p.exists());
            crate::cmd::remux::publish_no_overwrite(temp, &alt)?;
            Ok(alt)
        }
        Err(e) => Err(e),
    }
}

/// Remember partial files while their job runs, so a crash leaves a list to clean up.
pub fn register(registry: &Path, p: &Path) {
    let mut list = read_registry(registry);
    list.push(p.to_path_buf());
    write_registry(registry, &list);
}

pub fn unregister(registry: &Path, p: &Path) {
    let list: Vec<PathBuf> = read_registry(registry).into_iter().filter(|x| x != p).collect();
    write_registry(registry, &list);
}

fn read_registry(registry: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(registry).map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
}

fn write_registry(registry: &Path, list: &[PathBuf]) {
    if list.is_empty() {
        let _ = std::fs::remove_file(registry);
        return;
    }
    if let Some(d) = registry.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::write(registry, text);
}

fn old_enough(p: &Path, min_age: Duration) -> bool {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .map(|t| SystemTime::now().duration_since(t).unwrap_or_default() >= min_age)
        .unwrap_or(false)
}

/// Delete partial files left by a crash: everything in the registry plus any partial in
/// `dirs`, if not modified for `min_age` (a second running instance keeps writing to its own).
/// Returns the deleted paths.
pub fn cleanup_stale(registry: Option<&Path>, dirs: &[PathBuf], min_age: Duration) -> Vec<PathBuf> {
    let mut gone = vec![];
    let mut keep = vec![];
    if let Some(r) = registry {
        for p in read_registry(r) {
            if !p.exists() {
                continue;
            }
            if is_partial(&p) && old_enough(&p, min_age) && std::fs::remove_file(&p).is_ok() {
                gone.push(p);
            } else {
                keep.push(p);
            }
        }
        write_registry(r, &keep);
    }
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && is_partial(&p) && old_enough(&p, min_age) && std::fs::remove_file(&p).is_ok() {
                gone.push(p);
            }
        }
    }
    gone
}

/// "1.4 MB"
pub fn fmt_size(b: u64) -> String {
    const U: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} bytes");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", v, U[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }
    fn set(v: &[&str], ci: bool) -> HashSet<String> {
        v.iter().map(|s| path_key(Path::new(s), ci)).collect()
    }

    #[test]
    fn keep_both_naming() {
        let disk = set(&["/o/a.mp4", "/o/a (2).mp4", "/o/my.clip.v2.mkv", "/o/noext", "/o/x (2).mp4", "/o/x (3).mp4", "/o/視頻 ünï.mp4", "/o/.hidden"], false);
        let t = |p: &Path| disk.contains(&path_key(p, false));
        assert_eq!(keep_both_name(Path::new("/o/a.mp4"), &t), pb("/o/a (3).mp4"));
        assert_eq!(keep_both_name(Path::new("/o/b.mp4"), &t), pb("/o/b (2).mp4"));
        assert_eq!(keep_both_name(Path::new("/o/my.clip.v2.mkv"), &t), pb("/o/my.clip.v2 (2).mkv"));
        assert_eq!(keep_both_name(Path::new("/o/noext"), &t), pb("/o/noext (2)"));
        // an existing "(2)" continues numbering instead of producing "x (2) (2)"
        assert_eq!(keep_both_name(Path::new("/o/x (2).mp4"), &t), pb("/o/x (4).mp4"));
        assert_eq!(keep_both_name(Path::new("/o/視頻 ünï.mp4"), &t), pb("/o/視頻 ünï (2).mp4"));
        assert_eq!(keep_both_name(Path::new("/o/.hidden"), &t), pb("/o/.hidden (2)"));
        // "(0)" / "(abc)" / "(2)" alone are not counters
        assert_eq!(keep_both_name(Path::new("/o/v (0).mp4"), &t), pb("/o/v (0) (2).mp4"));
        assert_eq!(keep_both_name(Path::new("/o/v (abc).mp4"), &t), pb("/o/v (abc) (2).mp4"));
        assert_eq!(keep_both_name(Path::new("/o/(2).mp4"), &t), pb("/o/(2) (2).mp4"));
        assert_eq!(split_name("archive.tar.gz"), ("archive.tar", ".gz"));
        assert_eq!(split_name("trailingdot."), ("trailingdot.", ""));
    }

    #[test]
    fn case_insensitive_matching() {
        let disk = set(&[r"C:\Out\Clip.MP4", r"C:\Out\clip (2).mp4"], true);
        let t = |p: &Path| disk.contains(&path_key(p, true));
        assert_eq!(keep_both_name(Path::new(r"C:\out\clip.mp4"), &t), pb(r"C:\out\clip (3).mp4"));
        assert_eq!(path_key(Path::new("C:/Out/A.mp4"), true), path_key(Path::new(r"c:\out\a.MP4"), true));
        assert_ne!(path_key(Path::new("/o/A.mp4"), false), path_key(Path::new("/o/a.mp4"), false));
        // conflicts found case-insensitively, input == output detected
        let targets = vec![Target { output: pb(r"C:\v\IN.mkv"), inputs: vec![pb(r"c:\v\in.MKV")] }];
        let c = find_conflicts(&targets, &|_| None, true);
        assert_eq!(c[0].reasons, vec![Reason::SameAsInput]);
        assert!(!c[0].can_overwrite());
    }

    fn tgt(out: &str, ins: &[&str]) -> Target {
        Target { output: pb(out), inputs: ins.iter().map(|s| pb(s)).collect() }
    }

    #[test]
    fn conflicts_and_resolution() {
        let disk: HashMap<String, Existing> = [("/o/a.mp4", 10u64), ("/v/in.mp4", 99)]
            .iter()
            .map(|(p, s)| (path_key(Path::new(p), false), Existing { size: *s, modified: None }))
            .collect();
        let st = |p: &Path| disk.get(&path_key(p, false)).cloned();
        let ex = |p: &Path| disk.contains_key(&path_key(p, false));
        let targets = vec![
            tgt("/o/a.mp4", &["/v/1.mkv"]),  // exists
            tgt("/o/b.mp4", &["/v/2.mkv"]),  // free
            tgt("/o/b.mp4", &["/v/3.mkv"]),  // same as job 1
            tgt("/v/in.mp4", &["/v/in.mp4"]), // own input
            tgt("/o/a (2).mp4", &["/v/4.mkv"]), // free, but "a (2)" must not be handed to job 0
        ];
        let c = find_conflicts(&targets, &st, false);
        assert_eq!(c.iter().map(|c| c.job).collect::<Vec<_>>(), vec![0, 2, 3]);
        assert_eq!(c[0].existing().map(|e| e.size), Some(10));
        assert!(c[0].can_overwrite());
        assert_eq!(c[1].reasons, vec![Reason::SameAsJob(1)]);
        assert!(!c[1].can_overwrite());
        assert_eq!(c[2].reasons, vec![Reason::SameAsInput, Reason::Exists(Existing { size: 99, modified: None })]);

        let run = |o: &str, ow: bool| Outcome::Run { output: pb(o), overwrite: ow };
        // keep both everywhere
        let r = resolve(&targets, &|_| Choice::KeepBoth, &ex, false);
        assert_eq!(r, vec![run("/o/a (3).mp4", false), run("/o/b.mp4", false), run("/o/b (2).mp4", false), run("/v/in (2).mp4", false), run("/o/a (2).mp4", false)]);
        // overwrite: only the real existing file is replaced; the others are forced to a new name
        let r = resolve(&targets, &|_| Choice::Overwrite, &ex, false);
        assert_eq!(r, vec![run("/o/a.mp4", true), run("/o/b.mp4", false), run("/o/b (2).mp4", false), run("/v/in (2).mp4", false), run("/o/a (2).mp4", false)]);
        // skip
        let r = resolve(&targets, &|_| Choice::Skip, &ex, false);
        assert_eq!(r, vec![Outcome::Skip, run("/o/b.mp4", false), Outcome::Skip, Outcome::Skip, run("/o/a (2).mp4", false)]);
        // per-file: skip the first b.mp4 writer? (job 1 never conflicts) -> job 2 keeps b (2)
        let r = resolve(&targets, &|i| if i == 0 { Choice::Overwrite } else { Choice::Skip }, &ex, false);
        assert_eq!(r[0], run("/o/a.mp4", true));
        assert_eq!(r[2], Outcome::Skip);
    }

    #[test]
    fn setting_behaviour() {
        assert_eq!(WhenExists::default(), WhenExists::Ask);
        assert_eq!(WhenExists::Ask.initial_choice(), Choice::KeepBoth);
        assert_eq!(WhenExists::KeepBoth.initial_choice(), Choice::KeepBoth);
        assert_eq!(WhenExists::Overwrite.initial_choice(), Choice::Overwrite);
        assert_eq!(WhenExists::Skip.initial_choice(), Choice::Skip);
        let s: WhenExists = serde_json::from_str("\"Overwrite\"").unwrap();
        assert_eq!(s, WhenExists::Overwrite);
    }

    #[test]
    fn prepare_rewrites_output_and_flag() {
        let cmd = crate::cmd::BuiltCommand { args: ["-y", "-i", "/v/a b.mkv", "-c", "copy", "/o/a b.mp4"].iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let spec = JobSpec::new("Job", cmd, pb("/o/a b.mp4"), pb("/o/log.log"), None);
        assert_eq!(job_inputs(&spec), vec![pb("/v/a b.mkv")]);
        let j = prepare(spec.clone(), Path::new("/o/a b.mp4"), false, "7").unwrap();
        assert_eq!(j.args, ["-n", "-i", "/v/a b.mkv", "-c", "copy", "/o/a b.ffenc-partial-7.mp4"]);
        assert_eq!(j.post, PostAction::Publish { temp: pb("/o/a b.ffenc-partial-7.mp4"), dest: pb("/o/a b.mp4"), overwrite: false });
        assert_eq!(j.label, "Job");
        let j = prepare(spec, Path::new("/o/a b (2).mp4"), true, "8").unwrap();
        assert_eq!(j.args[0], "-y");
        assert_eq!(j.args.last().unwrap(), "/o/a b (2).ffenc-partial-8.mp4");
        assert!(j.label.contains("a b (2).mp4"));
        assert!(is_partial(Path::new("/o/a b (2).ffenc-partial-8.mp4")));
        assert!(!is_partial(Path::new("/o/a b.mp4")));
    }

    #[test]
    fn publish_and_cleanup_on_disk() {
        let d = std::env::temp_dir().join(format!("ffenc_guard_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        let dest = d.join("Out.mp4");
        std::fs::write(&dest, "old").unwrap();
        let t = d.join("Out.ffenc-partial-1.mp4");
        std::fs::write(&t, "new").unwrap();
        // not confirmed: existing file untouched, result lands in "Out (2).mp4"
        assert_eq!(publish(&t, &dest, false).unwrap(), d.join("Out (2).mp4"));
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old");
        std::fs::write(&t, "newer").unwrap();
        assert_eq!(publish(&t, &dest, true).unwrap(), dest);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "newer");
        assert!(!t.exists());
        // empty partial is an error
        std::fs::write(&t, "").unwrap();
        assert!(publish(&t, &d.join("e.mp4"), false).is_err());
        // registry + folder scan
        let reg = d.join("partials.txt");
        let p2 = d.join("x.ffenc-partial-2.mkv");
        std::fs::write(&p2, "x").unwrap();
        register(&reg, &t);
        register(&reg, &p2);
        assert!(cleanup_stale(Some(&reg), &[d.clone()], Duration::from_secs(3600)).is_empty(), "fresh partials kept");
        let gone = cleanup_stale(Some(&reg), &[d.clone()], Duration::ZERO);
        assert_eq!(gone.len(), 2);
        assert!(!reg.exists() && dest.exists());
        unregister(&reg, &t);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sizes() {
        assert_eq!(fmt_size(5), "5 bytes");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(3 * 1024 * 1024), "3.0 MB");
    }
}
