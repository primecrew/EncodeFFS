//! codecs branch: HEVC / AV1 / AC3 / audio copy end to end with the real ffmpeg. Each command comes
//! from `reencode::build` and the result is checked with ffprobe (codec, 10-bit pix_fmt, hvc1 tag,
//! AC3 5.1 kept). Encoders the local ffmpeg lacks are skipped with a note; no ffmpeg = skip.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffenc::cmd::codecs::{AudioCodec, Encoders, VideoCodec};
use ffenc::cmd::common::*;
use ffenc::cmd::reencode::{self, ReencodeParams};
use serde_json::Value;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn params(input: &Path, output: &Path, video: VideoEncode, audio: AudioEncode) -> ReencodeParams {
    ReencodeParams {
        input: input.into(),
        output: output.into(),
        duration_s: 1.0,
        trim_start: 0.0,
        trim_end: 0.0,
        passthrough: Passthrough::None,
        video,
        audio,
        range_convert: false,
        resize: None,
        deinterlace: false,
        chapters: None,
        faststart: true,
        subtitle_streams: vec![],
        has_attachments: false,
    }
}

fn run(ffmpeg: &Path, args: &[String]) {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(args).output().unwrap();
    assert!(out.status.success(), "{:?}\n{}", args, String::from_utf8_lossy(&out.stderr));
}

fn streams(ffprobe: &Path, f: &Path) -> Vec<Value> {
    let out = Command::new(ffprobe).args(["-v", "error", "-show_streams", "-of", "json"]).arg(f).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"].as_array().unwrap().clone()
}

fn get<'a>(s: &'a [Value], kind: &str) -> &'a Value {
    s.iter().find(|x| x["codec_type"] == kind).unwrap()
}

#[test]
fn hevc_av1_ac3_copy_with_real_ffmpeg() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else {
        eprintln!("ffmpeg/ffprobe not on PATH: skipped");
        return;
    };
    let enc = Encoders::from_list(&String::from_utf8_lossy(&Command::new(&ffmpeg).args(["-hide_banner", "-encoders"]).output().unwrap().stdout));
    let dir = std::env::temp_dir().join(format!("ffenc_codecs_{}", ffenc::util::unique_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("src clip ö.mkv");
    // 1 s, 8-bit video + 5.1 audio
    run(&ffmpeg, &["-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=1", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-filter_complex", "[1:a]pan=5.1|c0=c0|c1=c0|c2=c0|c3=c0|c4=c0|c5=c0[a]", "-map", "0:v", "-map", "[a]", "-c:v", "libx264", "-c:a", "aac", src.to_str().unwrap()].map(String::from));

    let mut done = vec![];
    if enc.libx265 {
        let v = VideoEncode { codec: VideoCodec::Hevc, crf: 28, preset: "ultrafast".into(), ..Default::default() };
        let out = dir.join("hevc 10.mp4");
        let b = reencode::build(&params(&src, &out, v, AudioEncode::default())).unwrap();
        run(&ffmpeg, &b.args);
        let s = streams(&ffprobe, &out);
        let vs = get(&s, "video");
        assert_eq!((vs["codec_name"].as_str(), vs["pix_fmt"].as_str(), vs["codec_tag_string"].as_str()), (Some("hevc"), Some("yuv420p10le"), Some("hvc1")));
        assert_eq!(get(&s, "audio")["codec_name"], "aac");
        done.push("hevc");
    } else {
        eprintln!("libx265 missing: HEVC not tested");
    }
    if enc.libsvtav1 {
        for (name, grain) in [("av1 10.mp4", 0), ("av1 grain.mkv", 8)] {
            let v = VideoEncode { codec: VideoCodec::Av1, crf: 40, svt_preset: 12, film_grain: grain, ..Default::default() };
            let au = AudioEncode { codec: AudioCodec::Ac3, ac3_bitrate_k: 384, source_channels: Some(6), ..Default::default() };
            let out = dir.join(name);
            let b = reencode::build(&params(&src, &out, v, au)).unwrap();
            run(&ffmpeg, &b.args);
            let s = streams(&ffprobe, &out);
            let vs = get(&s, "video");
            assert_eq!((vs["codec_name"].as_str(), vs["pix_fmt"].as_str()), (Some("av1"), Some("yuv420p10le")), "{name}");
            let a = get(&s, "audio");
            assert_eq!((a["codec_name"].as_str(), a["channels"].as_u64()), (Some("ac3"), Some(6)), "AC3 keeps 5.1 ({name})");
        }
        done.push("av1");
    } else {
        eprintln!("libsvtav1 missing: AV1 not tested");
    }
    // H.264 + keep original audio (copy): the AAC 5.1 track is copied untouched
    let out = dir.join("copy.mkv");
    let b = reencode::build(&params(&src, &out, VideoEncode { preset: "ultrafast".into(), ..Default::default() }, AudioEncode { codec: AudioCodec::Copy, ..Default::default() })).unwrap();
    run(&ffmpeg, &b.args);
    let s = streams(&ffprobe, &out);
    assert_eq!((get(&s, "video")["codec_name"].as_str(), get(&s, "video")["pix_fmt"].as_str()), (Some("h264"), Some("yuv420p")));
    assert_eq!((get(&s, "audio")["codec_name"].as_str(), get(&s, "audio")["channels"].as_u64()), (Some("aac"), Some(6)));
    eprintln!("tested: {done:?} + copy");
    std::fs::remove_dir_all(&dir).unwrap();
}
