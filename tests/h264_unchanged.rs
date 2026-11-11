//! codecs branch: H.264 (libx264) + AAC stays the default, and the commands the existing presets
//! build are byte-for-byte what 0.5.3 built. The expected strings were generated from the 0.5.3
//! code before any codec work (GOLDEN_WRITE=1 regenerates them into /tmp/golden for comparison).

use std::path::{Path, PathBuf};

use ffenc::cmd::batch::{self, BatchInput, BatchSettings};
use ffenc::cmd::common::*;
use ffenc::cmd::join::{self, AudioParams, JoinInput, JoinMethod, JoinParams, VideoParams};
use ffenc::cmd::reencode::{self, ReencodeParams, StreamRef};

fn re(out: &str) -> ReencodeParams {
    ReencodeParams {
        input: "/in/My Video.mkv".into(),
        output: out.into(),
        duration_s: 100.0,
        trim_start: 0.0,
        trim_end: 0.0,
        passthrough: Passthrough::None,
        video: VideoEncode::default(),
        audio: AudioEncode::default(),
        range_convert: false,
        resize: None,
        deinterlace: false,
        chapters: None,
        faststart: false,
        subtitle_streams: vec![],
        has_attachments: false,
    }
}

fn cases() -> Vec<(&'static str, String)> {
    let j = |a: &[String]| a.join(" | ");
    let mut v = vec![];
    v.push(("reencode_default_mkv", j(&reencode::build(&re("/out/a.mkv")).unwrap().args)));
    let mut p = re("/out/a.mp4");
    p.trim_start = 3.0;
    p.trim_end = 2.0;
    p.video = VideoEncode { crf: 19, preset: "veryslow".into(), tune: Some("grain".into()), ..Default::default() };
    p.audio.bitrate_k = 320;
    p.audio.gain_db = 3.0;
    p.audio.compressor = CompressorPreset::Medium;
    p.range_convert = true;
    p.resize = Some((1280, 720));
    p.deinterlace = true;
    p.faststart = true;
    p.subtitle_streams = vec![StreamRef { index: 2, codec: "subrip".into() }, StreamRef { index: 3, codec: "hdmv_pgs_subtitle".into() }];
    v.push(("reencode_everything_mp4", j(&reencode::build(&p).unwrap().args)));
    for pt in Passthrough::ALL {
        let mut p = re("/out/a.mp4");
        p.passthrough = pt;
        v.push(("reencode_passthrough", j(&reencode::build(&p).unwrap().args)));
    }
    let inputs = vec![BatchInput { path: "/in/x.mkv".into(), duration_s: 50.0, field_order: Some("tt".into()), ..Default::default() }, BatchInput { path: "/in/y z.mp4".into(), duration_s: 60.0, ..Default::default() }];
    for job in batch::plan(&inputs, &BatchSettings::default(), Path::new("/out")).unwrap() {
        v.push(("batch_default", j(&job.cmd.args)));
    }
    let ji = |n: &str| JoinInput {
        path: PathBuf::from(format!("/v/{n}")),
        duration_s: 10.0,
        video: Some(VideoParams { codec: "h264".into(), width: 1920, height: 1080, fps: Some(60.0), pix_fmt: Some("yuv420p".into()) }),
        audio: Some(AudioParams { codec: "aac".into(), sample_rate: Some(48000), channels: Some(2) }),
        audio_tracks: 1,
    };
    let jp = JoinParams {
        inputs: vec![ji("a.mp4"), ji("b.mp4")],
        output: "/o/j.mp4".into(),
        method: JoinMethod::Reencode,
        chapters_per_file: true,
        video: VideoEncode::default(),
        audio: AudioEncode::default(),
        faststart: true,
        list_file: "/tmp/l.txt".into(),
        chapter_file: "/tmp/c.ffmeta".into(),
    };
    v.push(("join_reencode", j(&join::build(&jp).unwrap().args)));
    v
}

#[test]
fn h264_commands_are_unchanged() {
    let got: Vec<String> = cases().into_iter().map(|(n, s)| format!("{n}: {s}")).collect();
    if std::env::var("GOLDEN_WRITE").is_ok() {
        std::fs::write("/tmp/golden/h264.txt", got.join("\n") + "\n").unwrap();
        return;
    }
    let want: Vec<&str> = GOLDEN.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(got, want);
}

const GOLDEN: &str = include_str!("h264_golden.txt");
