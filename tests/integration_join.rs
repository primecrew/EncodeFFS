//! Join / stitch end to end with the real ffmpeg: three clips whose names contain
//! spaces, apostrophes and unicode, in a folder with the same, joined with both
//! methods through the real job queue; duration and chapters checked with ffprobe.
//! Skips (passes) when ffmpeg is not on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ffenc::cmd::common::{AudioEncode, VideoEncode};
use ffenc::cmd::join::{self, JoinInput, JoinMethod, JoinParams};
use ffenc::ffmpeg::probe;
use ffenc::ffmpeg::runner::{JobSpec, JobStatus, Queue};
use serde_json::Value;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn ff(ffmpeg: &Path, dir: &Path, args: &[&str]) {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "generation failed: {:?}\n{}", args, String::from_utf8_lossy(&out.stderr));
}

fn probe_json(ffprobe: &Path, file: &Path) -> Value {
    let out = Command::new(ffprobe).args(["-v", "error", "-show_streams", "-show_chapters", "-show_format", "-of", "json"]).arg(file).output().unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn run(queue: &Queue, ffmpeg: &Path, spec: JobSpec) -> JobStatus {
    let ids = queue.enqueue(ffmpeg, vec![spec], None, true);
    let start = Instant::now();
    loop {
        let st = { queue.shared.lock().unwrap().jobs.iter().find(|j| j.id == ids[0]).unwrap().status.clone() };
        if st.finished() {
            return st;
        }
        assert!(start.elapsed() < Duration::from_secs(300), "timeout");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn chapters_of(v: &Value) -> Vec<(f64, f64, String)> {
    v["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["start_time"].as_str().unwrap().parse().unwrap(), c["end_time"].as_str().unwrap().parse().unwrap(), c["tags"]["title"].as_str().unwrap_or("").to_string()))
        .collect()
}

#[test]
fn join_three_clips_both_methods() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else {
        eprintln!("ffmpeg/ffprobe not on PATH – skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("ffenc join it's ünï {}", ffenc::util::unique_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let names = ["clip 10 o'brien & co.mp4", "clip 1 it's.mp4", "clip 2 ünï 視頻.mp4"];
    let lens = [2.0, 3.0, 4.0]; // after natural sort: clip 1 (3s), clip 2 (4s), clip 10 (2s)
    for (n, l) in names.iter().zip(lens) {
        let src = format!("testsrc=size=320x240:rate=25:duration={l}");
        let snd = format!("sine=frequency=440:duration={l}:sample_rate=48000");
        ff(&ffmpeg, &dir, &["-f", "lavfi", "-i", &src, "-f", "lavfi", "-i", &snd, "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ac", "2", n]);
    }
    // a mismatched clip for the re-encode join: other size, fps, rate, no audio
    ff(&ffmpeg, &dir, &["-f", "lavfi", "-i", "testsrc=size=160x90:rate=30:duration=2", "-c:v", "libx264", "-preset", "ultrafast", "odd 'one'.mp4"]);

    // the UI's folder-drop path: expand + natural sort
    let mut files: Vec<PathBuf> = ffenc::util::expand_dropped(&[dir.clone()]).into_iter().filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("clip")).collect();
    ffenc::util::natural_sort_paths(&mut files);
    let order: Vec<String> = files.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(order, ["clip 1 it's.mp4", "clip 2 ünï 視頻.mp4", "clip 10 o'brien & co.mp4"]);
    let inputs: Vec<JoinInput> = files.iter().map(|p| JoinInput::from_info(p, &probe::probe(&ffprobe, p).unwrap()).unwrap()).collect();
    assert!(join::compat_issues(&inputs).is_empty(), "{:?}", join::compat_issues(&inputs));
    let total = join::total_duration(&inputs);
    assert!((total - 9.0).abs() < 0.2, "{total}");

    let queue = Queue::new(None, || {});
    let out_dir = dir.join("out ünï");
    std::fs::create_dir_all(&out_dir).unwrap();
    for (method, ext, list) in [(JoinMethod::Copy, "mp4", inputs.clone()), (JoinMethod::Copy, "mkv", inputs.clone()), (JoinMethod::Reencode, "mkv", {
        let mut v = inputs.clone();
        let p = dir.join("odd 'one'.mp4");
        v.insert(1, JoinInput::from_info(&p, &probe::probe(&ffprobe, &p).unwrap()).unwrap());
        v
    })] {
        let output = out_dir.join(format!("joined it's {method:?}.{ext}"));
        let pr = JoinParams {
            inputs: list.clone(),
            output: output.clone(),
            method,
            chapters_per_file: true,
            video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None },
            audio: AudioEncode::default(),
            faststart: true,
            list_file: join::temp_list_path(),
            chapter_file: ffenc::cmd::chapters::temp_chapter_path(),
        };
        let cmd = join::build(&pr).unwrap();
        if method == JoinMethod::Reencode {
            assert!(cmd.warnings.iter().any(|w| w.contains("320x240 at 25 fps")), "{:?}", cmd.warnings);
        }
        let side: Vec<PathBuf> = cmd.side_files.iter().map(|f| f.0.clone()).collect();
        let log = out_dir.join(format!("log {method:?} {ext}.log"));
        let st = run(&queue, &ffmpeg, JobSpec::new("join", cmd, output.clone(), log.clone(), Some(pr.output_duration())));
        assert_eq!(st, JobStatus::Done, "{method:?} {ext}: {}", std::fs::read_to_string(&log).unwrap_or_default());
        for f in &side {
            assert!(!f.exists(), "temp file left behind: {}", f.display());
        }
        let v = probe_json(&ffprobe, &output);
        let dur: f64 = v["format"]["duration"].as_str().unwrap().parse().unwrap();
        let want = join::total_duration(&list);
        assert!((dur - want).abs() < 0.25, "{method:?} {ext}: duration {dur} vs {want}");
        let ch = chapters_of(&v);
        let titles: Vec<&str> = ch.iter().map(|c| c.2.as_str()).collect();
        let want_titles: Vec<String> = list.iter().map(|f| ffenc::util::stem(&f.path)).collect();
        assert_eq!(titles, want_titles, "{method:?} {ext}");
        let mut acc = 0.0;
        for (c, f) in ch.iter().zip(&list) {
            assert!((c.0 - acc).abs() < 0.01 && (c.1 - (acc + f.duration_s)).abs() < 0.01, "{method:?} {ext}: chapter {c:?} vs start {acc}");
            acc += f.duration_s;
        }
        let streams = v["streams"].as_array().unwrap();
        let vs: Vec<&Value> = streams.iter().filter(|s| s["codec_type"] == "video").collect();
        assert_eq!(vs.len(), 1);
        assert_eq!((vs[0]["width"].as_u64(), vs[0]["height"].as_u64()), (Some(320), Some(240)));
        assert_eq!(streams.iter().filter(|s| s["codec_type"] == "audio").count(), 1);
        eprintln!("{method:?} {ext}: {dur:.3}s, chapters {ch:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
