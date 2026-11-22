//! Output frame rate: validation, "same as source" leaves commands unchanged, old presets load,
//! and a real 30 fps -> 5 fps re-encode keeps the duration and audio (skips without ffmpeg).

use std::path::PathBuf;
use std::process::Command;

use ffenc::cmd::common::*;
use ffenc::cmd::reencode;

#[test]
fn validation() {
    for ok in ["5", " 5 ", "29.97", "30000/1001", "24000 / 1001", "0.5", "1000"] {
        assert!(parse_output_fps(ok).unwrap().is_some(), "{ok}");
    }
    assert_eq!(parse_output_fps("").unwrap(), None);
    assert_eq!(parse_output_fps("   ").unwrap(), None);
    assert_eq!(parse_output_fps("24000 / 1001").unwrap().unwrap(), "24000/1001");
    for bad in ["0", "0.0", "-5", "NaN", "nan", "inf", "1e3", "abc", "5fps", "30/0", "0/1", "-30/1", "1.2.3", "1001", "."] {
        assert!(parse_output_fps(bad).is_err(), "{bad} should be rejected");
    }
}

#[test]
fn same_as_source_unchanged_and_rate_added() {
    let v = VideoEncode::default();
    assert!(v.output_fps.is_empty());
    let base = x264_args(&v, Some("bwdif"), true);
    assert!(!base.iter().any(|a| a.contains("fps=")));
    let five = x264_args(&VideoEncode { output_fps: "5".into(), ..v.clone() }, Some("bwdif"), true);
    let i = five.iter().position(|a| a == "-vf").unwrap();
    assert_eq!(five[i + 1], "bwdif,fps=5", "fps last in the chain");
    assert_eq!(five.len(), base.len());
    let plain = x264_args(&VideoEncode { output_fps: "30000/1001".into(), ..VideoEncode::default() }, None, false);
    assert!(plain.windows(2).any(|w| w == ["-vf", "fps=30000/1001"]));
    assert!(!five.iter().any(|a| a == "-r"));
    // invalid never reaches the command (forms reject it before building)
    assert_eq!(x264_args(&VideoEncode { output_fps: "-1".into(), ..v }, None, false), x264_args(&VideoEncode::default(), None, false));
}

#[test]
fn presets_round_trip_and_old_presets_load() {
    let old: VideoEncode = serde_json::from_str(r#"{"crf": 20, "preset": "slow", "tune": null}"#).unwrap();
    assert_eq!(old.output_fps, "");
    let v = VideoEncode { output_fps: "5".into(), ..Default::default() };
    let j = serde_json::to_value(&v).unwrap();
    assert_eq!(j["output_fps"], "5");
    assert_eq!(serde_json::from_value::<VideoEncode>(j).unwrap(), v);
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn probe(ffprobe: &PathBuf, f: &PathBuf, what: &[&str]) -> String {
    let o = Command::new(ffprobe).args(["-v", "error"]).args(what).args(["-of", "default=nw=1:nk=1"]).arg(f).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

#[test]
fn thirty_to_five_keeps_duration_and_audio() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else { return };
    let dir = std::env::temp_dir().join(format!("ffenc_fps_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("meeting 30.mp4");
    let ok = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=4", "-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:v", "libx264", "-c:a", "aac", "-shortest"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(ok.success());
    let run = |fps: &str, name: &str| -> PathBuf {
        let out = dir.join(name);
        let pr = reencode::ReencodeParams {
            input: src.clone(),
            output: out.clone(),
            duration_s: 4.0,
            trim_start: 0.0,
            trim_end: 0.0,
            passthrough: Passthrough::None,
            video: VideoEncode { crf: 30, preset: "ultrafast".into(), output_fps: fps.into(), ..Default::default() },
            audio: AudioEncode::default(),
            range_convert: false,
            resize: None,
            deinterlace: false,
            chapters: None,
            faststart: true,
            subtitle_streams: vec![],
            has_attachments: false,
            picture: Default::default(),
        };
        let cmd = reencode::build(&pr).unwrap();
        assert_eq!(cmd.args.iter().any(|a| a.contains("fps=")), !fps.is_empty());
        let st = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(&cmd.args).output().unwrap();
        assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
        out
    };
    let five = run("5", "five.mp4");
    let same = run("", "same.mp4");
    let v = ["-select_streams", "v:0", "-show_entries", "stream=r_frame_rate"];
    assert_eq!(probe(&ffprobe, &five, &v), "5/1");
    assert_eq!(probe(&ffprobe, &same, &v), "30/1", "empty = same as source");
    let frames: u32 = probe(&ffprobe, &five, &["-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames"]).parse().unwrap();
    assert!((19..=21).contains(&frames), "4 s at 5 fps ≈ 20 frames, got {frames}");
    let dur = |f: &PathBuf| -> f64 { probe(&ffprobe, f, &["-show_entries", "format=duration"]).parse().unwrap() };
    assert!((dur(&five) - dur(&same)).abs() < 0.3, "duration kept (no speed-up): {} vs {}", dur(&five), dur(&same));
    let adur: f64 = probe(&ffprobe, &five, &["-select_streams", "a:0", "-show_entries", "stream=duration"]).parse().unwrap();
    assert!((adur - 4.0).abs() < 0.2, "audio untouched: {adur}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn codec_paths_add_fps_last() {
    use ffenc::cmd::codecs::{video_args, VideoCodec};
    let out = std::path::Path::new("/o/x.mp4");
    for codec in [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1] {
        let same = VideoEncode { codec, ..Default::default() };
        let a = video_args(&same, Some("bwdif"), false, out);
        assert!(a.windows(2).any(|w| w == ["-vf", "bwdif"]), "{codec:?}: same as source unchanged");
        let five = VideoEncode { output_fps: "5".into(), ..same };
        let a = video_args(&five, Some("bwdif"), false, out);
        assert!(a.windows(2).any(|w| w == ["-vf", "bwdif,fps=5"]), "{codec:?}: {a:?}");
        let a = video_args(&five, None, false, out);
        assert!(a.windows(2).any(|w| w == ["-vf", "fps=5"]), "{codec:?}: {a:?}");
    }
}

#[test]
fn hevc_thirty_to_five() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else { return };
    let dir = std::env::temp_dir().join(format!("ffenc_fps_hevc_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("in.mp4");
    assert!(Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=3"]).arg(&src).status().unwrap().success());
    let out = dir.join("out.mp4");
    let v = VideoEncode { codec: ffenc::cmd::codecs::VideoCodec::Hevc, crf: 30, preset: "ultrafast".into(), output_fps: "5".into(), ..Default::default() };
    let args = ffenc::cmd::codecs::video_args(&v, None, false, &out);
    let st = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&src).args(&args).arg(&out).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    assert_eq!(probe(&ffprobe, &out, &["-select_streams", "v:0", "-show_entries", "stream=codec_name,r_frame_rate"]), "hevc\n5/1");
    let _ = std::fs::remove_dir_all(&dir);
}
