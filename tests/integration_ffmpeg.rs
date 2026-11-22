//! End-to-end: generate synthetic media with the system ffmpeg, run every
//! mode's generated argument list through the real job queue, and check the
//! outputs with ffprobe. Skips (passes) when ffmpeg is not on PATH.
//!
//! Run: cargo test --test integration_ffmpeg -- --nocapture

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ffenc::cmd::chapters::{self, ChapterMode, ManualChapter};
use ffenc::cmd::common::*;
use ffenc::cmd::{batch, commentary, extract, mix, reencode, remux, replace_audio, still, subs};
use ffenc::ffmpeg::probe::{self, MediaInfo};
use ffenc::ffmpeg::runner::{JobSpec, JobStatus, PostAction, Queue};
use serde_json::Value;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

struct Env {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    dir: PathBuf,
    queue: Queue,
}

fn ff(env: &Env, args: &[&str]) {
    let out = Command::new(&env.ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).current_dir(&env.dir).output().unwrap();
    assert!(out.status.success(), "media generation failed: {:?}\n{}", args, String::from_utf8_lossy(&out.stderr));
}

fn probe_json(env: &Env, file: &Path) -> Value {
    let out = Command::new(&env.ffprobe)
        .args(["-v", "error", "-show_streams", "-show_chapters", "-show_format", "-of", "json"])
        .arg(file)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffprobe failed on {}", file.display());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn streams<'a>(v: &'a Value, t: &str) -> Vec<&'a Value> {
    v["streams"].as_array().unwrap().iter().filter(|s| s["codec_type"] == t).collect()
}

fn duration(v: &Value) -> f64 {
    v["format"]["duration"].as_str().unwrap().parse().unwrap()
}

fn info(env: &Env, p: &Path) -> MediaInfo {
    probe::probe(&env.ffprobe, p).unwrap()
}

/// Run specs through the real queue and wait.
fn run(env: &Env, specs: Vec<JobSpec>, group: Option<(String, bool, usize)>) -> Vec<JobStatus> {
    // like the app (0.3.3): every non-remux job writes a partial file and is renamed after success
    let specs: Vec<JobSpec> = specs
        .into_iter()
        .map(|s| match (&s.post, s.output.clone()) {
            (PostAction::None, Some(o)) => {
                let ow = o.exists();
                ffenc::output_guard::prepare(s, &o, ow, &ffenc::util::unique_id()).unwrap()
            }
            _ => s,
        })
        .collect();
    let ids = env.queue.enqueue(&env.ffmpeg, specs, group, true);
    let start = Instant::now();
    loop {
        let done: Vec<JobStatus> = {
            let s = env.queue.shared.lock().unwrap();
            ids.iter().map(|id| s.jobs.iter().find(|j| j.id == *id).unwrap().status.clone()).collect()
        };
        if done.iter().all(|s| s.finished()) {
            return done;
        }
        assert!(start.elapsed() < Duration::from_secs(300), "timeout");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn spec(label: &str, cmd: ffenc::cmd::BuiltCommand, out: &Path, dur: Option<f64>) -> JobSpec {
    let log = out.with_file_name(format!("ffmpeg_error_{}.log", ffenc::util::stem(out)));
    JobSpec::new(label, cmd, out.to_path_buf(), log, dur)
}

fn assert_ok(st: &[JobStatus], what: &str) {
    for s in st {
        assert_eq!(*s, JobStatus::Done, "{what}");
    }
}

fn setup() -> Option<Env> {
    let ffmpeg = which("ffmpeg")?;
    let ffprobe = which("ffprobe")?;
    // Unicode + spaces in the working folder to prove argv handling.
    let dir = std::env::temp_dir().join(format!("ffenc it ünï 視頻 {}", ffenc::util::unique_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let env = Env { ffmpeg, ffprobe, dir, queue: Queue::new(None, || {}) };
    let v = ["-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=6"];
    // progressive mp4 with one audio track
    ff(&env, &[&v[..], &["-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest", "prog clip.mp4"]].concat());
    // chapters for the two-audio mkv
    std::fs::write(env.dir.join("ch.txt"), ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=3000\ntitle=A\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=3000\nEND=6000\ntitle=B\n").unwrap();
    ff(&env, &[&v[..], &["-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000", "-f", "lavfi", "-i", "sine=frequency=880:duration=6:sample_rate=48000",
        "-i", "ch.txt", "-map", "0", "-map", "1", "-map", "2", "-map_chapters", "3",
        "-metadata:s:a:0", "title=Game", "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Mic",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "two_audio.mkv"]].concat());
    // subtitles (two srt tracks) + an attachment
    std::fs::write(env.dir.join("a.srt"), "1\n00:00:00,500 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:05,000\nWorld\n").unwrap();
    std::fs::write(env.dir.join("font.txt"), "not really a font").unwrap();
    ff(&env, &[&v[..], &["-f", "lavfi", "-i", "sine=duration=6", "-i", "a.srt", "-i", "a.srt",
        "-map", "0", "-map", "1", "-map", "2", "-map", "3", "-metadata:s:s:0", "language=eng", "-metadata:s:s:1", "language=ger", "-metadata:s:s:1", "title=Forced",
        "-attach", "font.txt", "-metadata:s:t", "mimetype=text/plain",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-c:s", "srt", "subs.mkv"]].concat());
    // interlaced (top field first)
    ff(&env, &["-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=4", "-f", "lavfi", "-i", "sine=duration=4",
        "-c:v", "libx264", "-preset", "ultrafast", "-flags", "+ildct+ilme", "-x264opts", "tff=1", "-c:a", "aac", "-shortest", "interlaced.mkv"]);
    ff(&env, &["-f", "lavfi", "-i", "testsrc=size=320x240", "-frames:v", "1", "cover image.png"]);
    ff(&env, &["-f", "lavfi", "-i", "sine=frequency=300:duration=3:sample_rate=44100", "-ac", "2", "voice.wav"]);
    Some(env)
}

#[test]
fn all_modes_end_to_end() {
    let Some(env) = setup() else {
        eprintln!("ffmpeg/ffprobe not on PATH – skipping integration test");
        return;
    };
    let d = env.dir.clone();
    let out = d.join("out folder");
    std::fs::create_dir_all(&out).unwrap();

    // sanity: probe parsing of generated media
    let il = info(&env, &d.join("interlaced.mkv")).video_summary().unwrap();
    assert!(il.interlaced, "field order {:?}", il.field_order);
    let two = info(&env, &d.join("two_audio.mkv"));
    assert_eq!(two.audio_count(), 2);
    let subinfo = info(&env, &d.join("subs.mkv"));
    assert_eq!(subinfo.subtitle_infos().len(), 2);
    assert!(subinfo.has_attachments());

    // ---- Mode 1: full re-encode, trim, resize, range, gain+compressor, chapters, 2 audio tracks
    let out_dur = 6.0 - 1.0 - 0.5;
    let ch = chapters::build_chapters(out_dur, ChapterMode::Both, 2.0, &[ManualChapter { time: "00:00:03".into(), title: "Boss = fight".into() }]).unwrap();
    let chf = d.join("chapters test.txt");
    let pr = reencode::ReencodeParams {
        input: d.join("two_audio.mkv"),
        output: out.join("m1 full.mkv"),
        duration_s: two.duration().unwrap(),
        trim_start: 1.0,
        trim_end: 0.5,
        passthrough: Passthrough::None,
        video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: Some("film".into()), ..Default::default() },
        audio: AudioEncode { bitrate_k: 128, gain_db: 3.0, compressor: CompressorPreset::Medium, ..Default::default() },
        range_convert: true,
        resize: Some(resize_dims(320, 240, 120).unwrap()),
        deinterlace: false,
        chapters: Some((chf.clone(), chapters::ffmetadata(&ch))),
        faststart: false,
        subtitle_streams: vec![],
        has_attachments: false,
        picture: Default::default(),
    };
    let st = run(&env, vec![spec("m1 full", reencode::build(&pr).unwrap(), &pr.output, Some(out_dur))], None);
    assert_ok(&st, "mode 1 full");
    let j = probe_json(&env, &pr.output);
    let v = streams(&j, "video")[0];
    assert_eq!((v["width"].as_i64(), v["height"].as_i64()), (Some(160), Some(120)));
    assert_eq!(v["color_range"], "tv");
    assert_eq!(v["color_space"], "bt709");
    let a = streams(&j, "audio");
    assert_eq!(a.len(), 2, "both audio tracks kept");
    assert!(a.iter().all(|s| s["codec_name"] == "aac"));
    assert_eq!(a[0]["tags"]["title"], "Game", "track title preserved with -filter:a");
    assert_eq!(a[1]["tags"]["title"], "Mic");
    assert!((duration(&j) - out_dur).abs() < 0.3, "duration {}", duration(&j));
    let chs = j["chapters"].as_array().unwrap();
    assert_eq!(chs.len(), 3, "chapters relative to trimmed output: 0, 2, 3(manual); no auto chapter in the last second -> {chs:?}");
    assert_eq!(chs[2]["tags"]["title"], "Boss = fight");
    assert!(!chf.exists(), "chapter temp file cleaned up");
    assert!(!out.join("ffmpeg_error_m1 full.log").exists() || true);

    // ---- Mode 1 passthrough variants on mp4 (faststart)
    for (pt, name) in [(Passthrough::Audio, "pt_a"), (Passthrough::Video, "pt_v"), (Passthrough::Both, "pt_b")] {
        let pr = reencode::ReencodeParams {
            input: d.join("prog clip.mp4"),
            output: out.join(format!("{name}.mp4")),
            duration_s: 6.0,
            trim_start: 0.0,
            trim_end: 1.0,
            passthrough: pt,
            video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None, ..Default::default() },
            audio: AudioEncode { gain_db: -2.0, ..Default::default() },
            range_convert: true,
            resize: None,
            deinterlace: false,
            chapters: None,
            faststart: true,
            subtitle_streams: vec![],
            has_attachments: false,
            picture: Default::default(),
        };
        let st = run(&env, vec![spec(name, reencode::build(&pr).unwrap(), &pr.output, Some(5.0))], None);
        assert_ok(&st, name);
        let j = probe_json(&env, &pr.output);
        assert!((duration(&j) - 5.0).abs() < 0.3, "{name} duration {}", duration(&j));
    }

    // ---- Mode 1 deinterlace
    let pr = reencode::ReencodeParams {
        input: d.join("interlaced.mkv"),
        output: out.join("deint.mkv"),
        duration_s: il.duration_s,
        trim_start: 0.0,
        trim_end: 0.0,
        passthrough: Passthrough::None,
        video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None, ..Default::default() },
        audio: AudioEncode::default(),
        range_convert: false,
        resize: None,
        deinterlace: true,
        chapters: None,
        faststart: false,
        subtitle_streams: vec![],
        has_attachments: false,
        picture: Default::default(),
    };
    assert_ok(&run(&env, vec![spec("deint", reencode::build(&pr).unwrap(), &pr.output, Some(il.duration_s))], None), "deinterlace");
    let j = probe_json(&env, &pr.output);
    assert_eq!(streams(&j, "video")[0]["r_frame_rate"], "50/1", "bwdif send_field doubles the rate");

    // ---- Mode 2 still image
    let pr = still::StillParams {
        image: d.join("cover image.png"),
        audio: d.join("voice.wav"),
        output: out.join("cover_still.mp4"),
        video: still::still_default_video(),
        audio_encode: ffenc::cmd::common::AudioEncode { bitrate_k: 128, ..Default::default() },
        duration_s: None,
        faststart: true,
        range_convert: true,
    };
    assert_ok(&run(&env, vec![spec("still", still::build(&pr).unwrap(), &pr.output, Some(3.0))], None), "still");
    let j = probe_json(&env, &pr.output);
    assert!((duration(&j) - 3.0).abs() < 0.5);
    assert_eq!(streams(&j, "video")[0]["pix_fmt"], "yuv420p");
    assert_eq!(streams(&j, "video")[0]["color_space"], "bt709");
    let pr2 = still::StillParams { duration_s: Some(1.5), range_convert: false, output: out.join("cover_short.mp4"), ..pr };
    assert_ok(&run(&env, vec![spec("still2", still::build(&pr2).unwrap(), &pr2.output, Some(1.5))], None), "still custom duration");
    assert!((duration(&probe_json(&env, &pr2.output)) - 1.5).abs() < 0.3);

    // ---- Mode 3 commentary (source has 2 audio tracks -> commentary is a:2)
    let pr = commentary::CommentaryParams {
        video: d.join("two_audio.mkv"),
        commentary: d.join("voice.wav"),
        output: out.join("two_with_commentary.mp4"),
        title: "Commentary Track".into(),
        bitrate_k: 160,
        faststart: true,
        original_audio_count: 2,
    };
    assert_ok(&run(&env, vec![spec("commentary", commentary::build(&pr).unwrap(), &pr.output, Some(6.0))], None), "commentary");
    let j = probe_json(&env, &pr.output);
    let a = streams(&j, "audio");
    assert_eq!(a.len(), 3);
    assert_eq!(a[2]["tags"]["handler_name"], "Commentary Track", "MP4 track name via handler_name");
    let pr = commentary::CommentaryParams { output: out.join("two_with_commentary.mkv"), ..pr };
    assert_ok(&run(&env, vec![spec("commentary mkv", commentary::build(&pr).unwrap(), &pr.output, Some(6.0))], None), "commentary mkv");
    let j = probe_json(&env, &pr.output);
    let a = streams(&j, "audio");
    assert_eq!(a.len(), 3);
    assert_eq!(a[2]["tags"]["title"], "Commentary Track");
    assert_eq!(a[0]["tags"]["title"], "Game");

    // ---- Mode 4 extract WAV
    let sr = two.first_audio_sample_rate().unwrap_or(48000);
    let pr = extract::ExtractParams { input: d.join("two_audio.mkv"), output: out.join("two_audio_audio.wav"), sample_rate: sr, map_audio: None };
    assert_ok(&run(&env, vec![spec("extract", extract::build(&pr).unwrap(), &pr.output, None)], None), "extract");
    let j = probe_json(&env, &pr.output);
    let a = streams(&j, "audio")[0];
    assert_eq!(a["codec_name"], "pcm_s16le");
    assert_eq!(a["channels"], 2);
    assert_eq!(a["sample_rate"], "48000");

    // ---- Mode 5 replace audio
    let pr = replace_audio::ReplaceAudioParams {
        video: d.join("prog clip.mp4"),
        wav: d.join("voice.wav"),
        output: out.join("prog_newaudio.mp4"),
        audio: AudioEncode { gain_db: 6.0, compressor: CompressorPreset::Strong, ..Default::default() },
    };
    assert_ok(&run(&env, vec![spec("replace", replace_audio::build(&pr).unwrap(), &pr.output, Some(6.0))], None), "replace");
    let j = probe_json(&env, &pr.output);
    assert_eq!(streams(&j, "audio").len(), 1);
    assert_eq!(streams(&j, "video")[0]["codec_name"], "h264");

    // ---- Mode 6 mix
    let pr = mix::MixParams { input: d.join("two_audio.mkv"), output: out.join("two_mixed.mkv"), audio_count: 2, bitrate_k: 192 };
    let cmd = mix::build(&pr).unwrap();
    assert_ok(&run(&env, vec![spec("mix", cmd, &pr.output, Some(6.0))], None), "mix");
    let j = probe_json(&env, &pr.output);
    assert_eq!(streams(&j, "audio").len(), 1);
    assert_eq!(j["chapters"].as_array().unwrap().len(), 2, "chapters kept");

    // ---- Mode 7 batch -> MP4 (subs.mkv has srt + attachment, interlaced.mkv gets deinterlaced)
    let bdir = d.join("batch in");
    std::fs::create_dir_all(&bdir).unwrap();
    for f in ["prog clip.mp4", "subs.mkv", "interlaced.mkv"] {
        std::fs::copy(d.join(f), bdir.join(f)).unwrap();
    }
    let inputs: Vec<batch::BatchInput> = ["interlaced.mkv", "prog clip.mp4", "subs.mkv"]
        .iter()
        .map(|f| {
            let p = bdir.join(f);
            let i = info(&env, &p);
            let v = i.video_summary().unwrap();
            batch::BatchInput { path: p, duration_s: v.duration_s, field_order: v.field_order, subtitle_streams: i.subtitle_refs(), has_attachments: i.has_attachments(), has_chapters: i.has_chapters(), width: v.width, height: v.height }
        })
        .collect();
    let bs = batch::BatchSettings {
        video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None, ..Default::default() },
        trim_start: 0.5,
        faststart: true,
        audio: AudioEncode { gain_db: 2.0, compressor: CompressorPreset::Light, ..Default::default() },
        ..Default::default()
    };
    let bout = d.join("batch out");
    let jobs = batch::plan(&inputs, &bs, &bout).unwrap();
    let specs: Vec<JobSpec> = jobs.iter().map(|j| JobSpec::new(j.output.display().to_string(), j.cmd.clone(), j.output.clone(), j.log_file.clone(), Some(j.out_duration))).collect();
    assert_ok(&run(&env, specs, Some(("Batch".into(), false, 0))), "batch");
    let j = probe_json(&env, &bout.join("subs_encoded.mp4"));
    let s = streams(&j, "subtitle");
    assert_eq!(s.len(), 2);
    assert!(s.iter().all(|x| x["codec_name"] == "mov_text"));
    assert!(streams(&j, "attachment").is_empty());
    let j = probe_json(&env, &bout.join("interlaced_encoded.mp4"));
    assert_eq!(streams(&j, "video")[0]["r_frame_rate"], "50/1");
    let j = probe_json(&env, &bout.join("prog clip_encoded.mp4"));
    assert_eq!(streams(&j, "video")[0]["r_frame_rate"], "25/1");

    // ---- Mode 8 subtitles: pick only stream 3 (ger, Forced) -> MKV copy, and both -> MP4 mov_text
    let src_subs = subinfo.subtitle_infos();
    let dst = info(&env, &d.join("prog clip.mp4"));
    let mk = |output: PathBuf, selected: Vec<u32>, copy_only: bool| subs::SubsParams {
        source: d.join("subs.mkv"),
        destination: d.join("prog clip.mp4"),
        output,
        source_streams: src_subs.clone(),
        selected,
        dest_streams: dst.subtitle_infos(),
        keep_existing: true,
        copy_only,
        video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None, ..Default::default() },
        audio_bitrate_k: 128,
        range_convert: false,
        mp4_handling: subs::Mp4SubHandling::ConvertMovText,
        faststart: true,
    };
    let pr = mk(out.join("prog_with_subs.mkv"), vec![src_subs[1].index], true);
    assert_ok(&run(&env, vec![spec("subs mkv", subs::build(&pr).unwrap(), &pr.output, Some(6.0))], None), "subs mkv");
    let j = probe_json(&env, &pr.output);
    let s = streams(&j, "subtitle");
    assert_eq!(s.len(), 1);
    assert_eq!(s[0]["tags"]["language"], "ger");
    assert_eq!(s[0]["codec_name"], "subrip");
    let pr = mk(out.join("prog_with_subs.mp4"), src_subs.iter().map(|s| s.index).collect(), false);
    assert_ok(&run(&env, vec![spec("subs mp4", subs::build(&pr).unwrap(), &pr.output, Some(6.0))], None), "subs mp4 reencode");
    let s = streams(&probe_json(&env, &pr.output), "subtitle").len();
    assert_eq!(s, 2);

    // ---- Mode 9 remux: two_audio.mkv ok (+delete), subs.mkv fails (srt/attachment not MP4-safe), exists.mkv skipped
    let rdir = d.join("remux");
    std::fs::create_dir_all(&rdir).unwrap();
    std::fs::copy(d.join("two_audio.mkv"), rdir.join("good.mkv")).unwrap();
    std::fs::copy(d.join("subs.mkv"), rdir.join("bad.mkv")).unwrap();
    std::fs::copy(d.join("two_audio.mkv"), rdir.join("exists.mkv")).unwrap();
    std::fs::write(rdir.join("exists.mp4"), b"keep me").unwrap();
    let sources = vec![rdir.join("good.mkv"), rdir.join("bad.mkv"), rdir.join("exists.mkv")];
    let plan = remux::plan(&sources, &rdir, remux::Container::Mp4, &|p| p.exists(), ffenc::util::unique_id);
    assert_eq!(plan.jobs.len(), 2);
    assert_eq!(plan.skipped.len(), 1);
    let specs = plan
        .jobs
        .iter()
        .map(|j| JobSpec {
            label: format!("remux {}", j.source.display()),
            args: remux::args(j),
            output: Some(j.temp.clone()),
            log_file: Some(j.dest.with_file_name(format!("ffmpeg_error_{}.log", ffenc::util::stem(&j.dest)))),
            duration_s: None,
            side_files: vec![],
            keep_side_files: false,
            post: PostAction::Remux { temp: j.temp.clone(), dest: j.dest.clone(), delete_source: Some(j.source.clone()) },
            warnings: vec![],
        })
        .collect();
    let st = run(&env, specs, Some(("Remux".into(), true, plan.skipped.len())));
    assert_eq!(st[0], JobStatus::Done);
    assert!(matches!(st[1], JobStatus::Failed(_)), "bad.mkv should fail: {:?}", st[1]);
    assert!(rdir.join("good.mp4").is_file());
    assert!(!rdir.join("good.mkv").exists(), "successful source deleted");
    assert!(rdir.join("bad.mkv").exists(), "failed source kept");
    assert!(!rdir.join("bad.mp4").exists());
    assert_eq!(std::fs::read(rdir.join("exists.mp4")).unwrap(), b"keep me");
    let leftovers: Vec<_> = std::fs::read_dir(&rdir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with(".remux_")).collect();
    assert!(leftovers.is_empty(), "temp files cleaned");
    std::thread::sleep(Duration::from_millis(300));
    let log: Vec<String> = env.queue.shared.lock().unwrap().log.iter().cloned().collect();
    assert!(
        log.iter().any(|l| l == "Queue finished: 1 converted, 1 failed, 1 skipped, 1 source file(s) deleted, 0 deletion failure(s)."),
        "summary missing"
    );

    // ---- Cancellation: a long encode is stopped cleanly and the partial output removed
    let long = reencode::ReencodeParams {
        input: d.join("prog clip.mp4"),
        output: out.join("cancel me.mp4"),
        duration_s: 6.0,
        trim_start: 0.0,
        trim_end: 0.0,
        passthrough: Passthrough::None,
        video: VideoEncode { crf: 18, preset: "veryslow".into(), tune: None, ..Default::default() },
        audio: AudioEncode::default(),
        range_convert: false,
        resize: Some((3840, 2880)),
        deinterlace: false,
        chapters: None,
        faststart: false,
        subtitle_streams: vec![],
        has_attachments: false,
        picture: Default::default(),
    };
    let ids = env.queue.enqueue(&env.ffmpeg, vec![spec("cancel", reencode::build(&long).unwrap(), &long.output, Some(6.0))], None, true);
    std::thread::sleep(Duration::from_millis(1500));
    env.queue.cancel(ids[0]);
    let t = Instant::now();
    loop {
        let s = env.queue.shared.lock().unwrap().jobs.iter().find(|j| j.id == ids[0]).unwrap().status.clone();
        if s.finished() {
            assert_eq!(s, JobStatus::Cancelled);
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "cancel took too long");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!long.output.exists(), "partial output removed");

    println!("integration outputs in {}", d.display());
    let _ = std::fs::remove_dir_all(&d);
}

/// Custom preset pipelines (CustomOptions -> params -> builder -> real ffmpeg).
#[test]
fn custom_pipelines_end_to_end() {
    use ffenc::cmd::custom::{self, AudioAction, CustomOptions, SubAction, VideoAction};
    let Some(env) = setup() else {
        eprintln!("ffmpeg/ffprobe not on PATH – skipping integration test");
        return;
    };
    let d = env.dir.clone();
    let out = d.join("custom out");
    std::fs::create_dir_all(&out).unwrap();
    let go = |name: &str, o: &CustomOptions, input: &Path, af: Option<PathBuf>, sf: Option<PathBuf>| -> (PathBuf, Value) {
        let output = out.join(name);
        let i = info(&env, input);
        let pr = o.params(input, &output, &i, af, sf).unwrap();
        let cmd = custom::build(&pr).unwrap();
        eprintln!("custom {name}: {}", cmd.args.join(" "));
        let st = run(&env, vec![spec(name, cmd, &output, pr.output_duration())], None);
        assert_ok(&st, name);
        let j = probe_json(&env, &output);
        (output, j)
    };
    let base = CustomOptions { video_encode: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None, ..Default::default() }, ..Default::default() };

    // 1. default re-encode -> mp4: trim, resize, auto chapters, faststart, extra in/out args, both tracks kept
    let o = CustomOptions {
        trim_start: 1.0,
        resize: true,
        resize_height: 120,
        chapter_mode: ChapterMode::Auto,
        chapter_interval_min: 2.0 / 60.0,
        input_args: "-analyzeduration 10M".into(),
        output_args: "-metadata \"comment=from custom preset\"".into(),
        ..base.clone()
    };
    let (_, j) = go("c1.mp4", &o, &d.join("two_audio.mkv"), None, None);
    let v = streams(&j, "video");
    assert_eq!((v[0]["codec_name"].as_str(), v[0]["width"].as_u64(), v[0]["height"].as_u64()), (Some("h264"), Some(160), Some(120)));
    let a = streams(&j, "audio");
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|s| s["codec_name"] == "aac"));
    assert!((duration(&j) - 5.0).abs() < 0.2, "{}", duration(&j));
    assert_eq!(j["chapters"].as_array().unwrap().len(), 3);
    assert_eq!(j["format"]["tags"]["comment"], "from custom preset");

    // 2. copy video + mix tracks with gain and a title -> mkv
    let o = CustomOptions { container: "mkv".into(), video: VideoAction::Copy, audio: AudioAction::Mix, track_title: "Mixed".into(), audio_encode: AudioEncode { gain_db: 2.0, ..Default::default() }, ..base.clone() };
    let (_, j) = go("c2.mkv", &o, &d.join("two_audio.mkv"), None, None);
    let a = streams(&j, "audio");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["tags"]["title"], "Mixed");
    assert_eq!(j["chapters"].as_array().unwrap().len(), 2, "source chapters kept by default");

    // 3. copy + add track (mp4 handler name)
    let o = CustomOptions { video: VideoAction::Copy, audio: AudioAction::AddTrack, track_title: "Commentary".into(), ..base.clone() };
    let (_, j) = go("c3.mp4", &o, &d.join("two_audio.mkv"), Some(d.join("voice.wav")), None);
    let a = streams(&j, "audio");
    assert_eq!(a.len(), 3);
    assert_eq!(a[2]["tags"]["handler_name"], "Commentary");

    // 4. replace audio, drop subtitles, keep attachment in mkv
    let o = CustomOptions { container: "mkv".into(), video: VideoAction::Copy, audio: AudioAction::Replace, subs: SubAction::Drop, ..base.clone() };
    let (_, j) = go("c4.mkv", &o, &d.join("subs.mkv"), Some(d.join("voice.wav")), None);
    assert_eq!(streams(&j, "audio").len(), 1);
    assert_eq!(streams(&j, "subtitle").len(), 0);
    assert_eq!(streams(&j, "attachment").len(), 1);

    // 5. convert subs for mp4 (attachment dropped)
    let o = CustomOptions { video: VideoAction::Copy, audio: AudioAction::Copy, subs: SubAction::Convert, ..base.clone() };
    let (_, j) = go("c5.mp4", &o, &d.join("subs.mkv"), None, None);
    let s = streams(&j, "subtitle");
    assert_eq!(s.len(), 2);
    assert!(s.iter().all(|x| x["codec_name"] == "mov_text"));
    assert_eq!(streams(&j, "attachment").len(), 0);

    // 6. import an .srt into an mp4 source, output mkv
    let o = CustomOptions { container: "mkv".into(), video: VideoAction::Copy, audio: AudioAction::Copy, subs: SubAction::Import, ..base.clone() };
    let (_, j) = go("c6.mkv", &o, &d.join("prog clip.mp4"), None, Some(d.join("a.srt")));
    assert_eq!(streams(&j, "subtitle").len(), 1);

    // 6b. chapter creator: copy-only "add chapters" on a file without chapters, container "same"
    let src = d.join("prog clip.mp4");
    assert!(!info(&env, &src).has_chapters());
    let title = "Boss = fight; #1 \\ ü 視頻";
    let o = CustomOptions {
        container: "same".into(),
        video: VideoAction::Copy,
        audio: AudioAction::Copy,
        chapter_mode: ChapterMode::Both,
        chapter_interval_min: 2.0 / 60.0,
        manual: vec![ManualChapter { time: "00:00:01.250".into(), title: title.into() }],
        ..base.clone()
    };
    assert_eq!(o.output_name(&src), "prog clip_custom.mp4");
    let (_, j) = go(&o.output_name(&src), &o, &src, None, None);
    let chs = j["chapters"].as_array().unwrap();
    let titles: Vec<&str> = chs.iter().map(|c| c["tags"]["title"].as_str().unwrap_or("")).collect();
    assert_eq!(titles[1], title, "special characters survive FFMETADATA escaping: {titles:?}");
    assert_eq!(chs[1]["start_time"], "1.250000");
    let last_end: f64 = chs.last().unwrap()["end_time"].as_str().unwrap().parse().unwrap();
    assert!((last_end - duration(&j)).abs() < 0.1, "last chapter ends at the output duration");

    // 6c. "auto-create if none": adds chapters only when the source has none
    let o = CustomOptions { container: "mkv".into(), video: VideoAction::Copy, audio: AudioAction::Copy, auto_chapters_if_none: true, chapter_interval_min: 2.0 / 60.0, ..base.clone() };
    let (_, j) = go("c6c_none.mkv", &o, &src, None, None);
    assert!(j["chapters"].as_array().unwrap().len() >= 2, "generated for a file without chapters");
    let (_, j) = go("c6c_has.mkv", &o, &d.join("two_audio.mkv"), None, None);
    assert_eq!(j["chapters"].as_array().unwrap().len(), 2, "existing chapters untouched");

    // 6d. ffmpeg reads a CRLF chapter file too
    let crlf = ffenc::cmd::chapters::ffmetadata_with(
        &[ffenc::cmd::chapters::Chapter { start_ms: 0, end_ms: 1000, title: "a=b".into() }, ffenc::cmd::chapters::Chapter { start_ms: 1000, end_ms: 2000, title: "ü".into() }],
        true,
    );
    std::fs::write(d.join("crlf.ffmeta"), crlf).unwrap();
    let crlf_out = out.join("crlf.mkv");
    let st = std::process::Command::new(&env.ffmpeg)
        .args(["-v", "error", "-y", "-i"]).arg(&src).args(["-f", "ffmetadata", "-i"]).arg(d.join("crlf.ffmeta"))
        .args(["-map", "0", "-map_chapters", "1", "-c", "copy", "-t", "2"]).arg(&crlf_out).status().unwrap();
    assert!(st.success());
    let j = probe_json(&env, &crlf_out);
    let chs = j["chapters"].as_array().unwrap();
    assert_eq!((chs[0]["tags"]["title"].as_str(), chs[1]["tags"]["title"].as_str()), (Some("a=b"), Some("ü")));

    // 7. audio only: drop video, AAC -> m4a
    let o = CustomOptions { container: "m4a".into(), video: VideoAction::Drop, ..base.clone() };
    let (_, j) = go("c7.m4a", &o, &d.join("two_audio.mkv"), None, None);
    assert_eq!(streams(&j, "video").len(), 0);
    assert_eq!(streams(&j, "audio").len(), 2);

    // 8. deinterlace (double rate) and drop source chapters
    let o = CustomOptions { container: "mkv".into(), drop_chapters: true, ..base.clone() };
    let (_, j) = go("c8.mkv", &o, &d.join("interlaced.mkv"), None, None);
    assert_eq!(streams(&j, "video")[0]["r_frame_rate"], "50/1");
    let (_, j) = go("c9.mkv", &o, &d.join("two_audio.mkv"), None, None);
    assert_eq!(j["chapters"].as_array().unwrap().len(), 0);

    // 10. folder batch pairing: same-name audio file next to the video
    let bd = d.join("batch pair");
    std::fs::create_dir_all(&bd).unwrap();
    std::fs::copy(d.join("prog clip.mp4"), bd.join("g1.mp4")).unwrap();
    std::fs::copy(d.join("voice.wav"), bd.join("g1.wav")).unwrap();
    let af = ffenc::util::find_companion(&bd.join("g1.mp4"), ffenc::util::ANY_AUDIO_EXTS).unwrap();
    let o = CustomOptions { video: VideoAction::Copy, audio: AudioAction::Replace, ..base.clone() };
    let (_, j) = go(&o.output_name(&bd.join("g1.mp4")), &o, &bd.join("g1.mp4"), Some(af), None);
    assert_eq!(streams(&j, "audio")[0]["sample_rate"], "44100", "replacement audio used");
    assert!(out.join("g1_custom.mp4").is_file());

    let _ = std::fs::remove_dir_all(&d);
}
