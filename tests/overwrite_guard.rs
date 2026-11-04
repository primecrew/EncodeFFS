//! End-to-end overwrite protection with the real ffmpeg + job queue:
//! an existing output survives a failed or cancelled job, is replaced only when the
//! overwrite was confirmed, and "keep both" writes "name (2).ext".
//! Skips (passes) when ffmpeg is not on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ffenc::cmd::BuiltCommand;
use ffenc::ffmpeg::runner::{JobSpec, JobStatus, Queue};
use ffenc::output_guard::{self, Choice, Outcome, Target};

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

const ORIGINAL: &[u8] = b"ORIGINAL - must not be destroyed";

fn spec(input: &Path, out: &Path, extra: &[&str]) -> JobSpec {
    let mut args: Vec<String> = vec!["-y".into()];
    args.extend(extra.iter().map(|s| s.to_string()));
    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    args.extend(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"].iter().map(|s| s.to_string()));
    args.push(out.to_string_lossy().into_owned());
    let log = out.with_file_name("ffmpeg_error_test.log");
    JobSpec::new("test", BuiltCommand { args, ..Default::default() }, out.to_path_buf(), log, None)
}

/// What the confirmation does: resolve the user's choice and prepare the job.
fn guarded(s: JobSpec, choice: Choice) -> Option<JobSpec> {
    let t = vec![Target { output: s.output.clone().unwrap(), inputs: output_guard::job_inputs(&s) }];
    match &output_guard::resolve(&t, &|_| choice, &|p| p.exists(), output_guard::CASE_INSENSITIVE)[0] {
        Outcome::Skip => None,
        Outcome::Run { output, overwrite } => Some(output_guard::prepare(s, output, *overwrite, &ffenc::util::unique_id()).unwrap()),
    }
}

fn wait(q: &Queue, id: u64) -> JobStatus {
    let t0 = Instant::now();
    loop {
        let st = q.shared.lock().unwrap().jobs.iter().find(|j| j.id == id).unwrap().status.clone();
        if st.finished() {
            return st;
        }
        assert!(t0.elapsed() < Duration::from_secs(120), "timeout");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn partials(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| output_guard::is_partial(p)).collect()
}

fn is_video(ffprobe: &Path, p: &Path) -> bool {
    Command::new(ffprobe).args(["-v", "error", "-show_streams"]).arg(p).output().map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("codec_type=video")).unwrap_or(false)
}

#[test]
fn existing_output_is_protected() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else {
        eprintln!("ffmpeg not on PATH – skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("ffenc guard ünï {}", ffenc::util::unique_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in clip.mkv");
    let ok = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=2", "-f", "lavfi", "-i", "sine=duration=2", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"])
        .arg(&input)
        .status()
        .unwrap();
    assert!(ok.success());
    let out = dir.join("Out Video.mp4");
    std::fs::write(&out, ORIGINAL).unwrap();
    let q = Queue::new(None, || {});
    let enqueue = |s: JobSpec| q.enqueue(&ffmpeg, vec![s], None, false)[0];

    // 1. confirmed overwrite, but ffmpeg fails (missing input): the old file survives
    let s = guarded(spec(&dir.join("missing.mkv"), &out, &[]), Choice::Overwrite).unwrap();
    assert_eq!(s.args[0], "-y");
    assert!(matches!(wait(&q, enqueue(s)), JobStatus::Failed(_)));
    assert_eq!(std::fs::read(&out).unwrap(), ORIGINAL, "failed job must not touch the existing file");
    assert!(partials(&dir).is_empty());

    // 2. confirmed overwrite, cancelled mid-encode (-re = real time, 60 s source)
    let s = guarded(spec(Path::new("testsrc=size=640x480:rate=25:duration=60"), &out, &["-re", "-f", "lavfi"]), Choice::Overwrite).unwrap();
    let id = enqueue(s);
    let t0 = Instant::now();
    while partials(&dir).is_empty() {
        assert!(t0.elapsed() < Duration::from_secs(30), "partial file never appeared");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    q.cancel(id);
    assert_eq!(wait(&q, id), JobStatus::Cancelled);
    assert_eq!(std::fs::read(&out).unwrap(), ORIGINAL, "cancelled job must not touch the existing file");
    assert!(partials(&dir).is_empty(), "partial deleted on cancel");

    // 3. keep both -> "Out Video (2).mp4", original untouched
    let s = guarded(spec(&input, &out, &[]), Choice::KeepBoth).unwrap();
    assert_eq!(s.args[0], "-n");
    let id = enqueue(s);
    assert_eq!(wait(&q, id), JobStatus::Done);
    let two = dir.join("Out Video (2).mp4");
    assert!(is_video(&ffprobe, &two), "keep both wrote (2)");
    // the "Encoding Finished" popup gets the final name, not the temporary partial name
    let fin = q.shared.lock().unwrap().jobs.iter().find(|j| j.id == id).unwrap().final_output.clone();
    assert_eq!(fin.as_deref(), Some(two.as_path()));
    assert_eq!(std::fs::read(&out).unwrap(), ORIGINAL);

    // 4. skip -> nothing queued
    assert!(guarded(spec(&input, &out, &[]), Choice::Skip).is_none());

    // 5. not confirmed, but the file "appears" during the run (prepared before it existed): never replaced
    let fresh = dir.join("race.mp4");
    let s = guarded(spec(&input, &fresh, &[]), Choice::KeepBoth).unwrap();
    std::fs::write(&fresh, ORIGINAL).unwrap();
    assert_eq!(wait(&q, enqueue(s)), JobStatus::Done);
    assert_eq!(std::fs::read(&fresh).unwrap(), ORIGINAL);
    assert!(is_video(&ffprobe, &dir.join("race (2).mp4")));

    // 6. output == input is never overwritten, even when "overwrite" is chosen
    let s = guarded(spec(&input, &input, &[]), Choice::Overwrite).unwrap();
    assert_eq!(s.output.as_deref(), Some(dir.join("in clip (2).mkv").as_path()));

    // 7. confirmed overwrite that succeeds replaces the file
    let s = guarded(spec(&input, &out, &[]), Choice::Overwrite).unwrap();
    assert_eq!(wait(&q, enqueue(s)), JobStatus::Done);
    assert!(is_video(&ffprobe, &out), "replaced after confirmation");
    assert!(partials(&dir).is_empty());

    // 8. "Keep temporary files for debugging": a cancelled partial is kept and logged
    q.set_keep_temp(true);
    let s = guarded(spec(Path::new("testsrc=size=640x480:rate=25:duration=60"), &dir.join("dbg.mp4"), &["-re", "-f", "lavfi"]), Choice::KeepBoth).unwrap();
    let id = enqueue(s);
    let t0 = Instant::now();
    while partials(&dir).is_empty() {
        assert!(t0.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    q.cancel(id);
    assert_eq!(wait(&q, id), JobStatus::Cancelled);
    assert_eq!(partials(&dir).len(), 1, "partial kept for debugging");
    assert!(q.shared.lock().unwrap().log.iter().any(|l| l.contains("Kept partial output (debugging)")));

    std::fs::remove_dir_all(&dir).unwrap();
}
