//! Frameserver (AviSynth script) preset.
//!
//! Part 1 (always, needs ffmpeg on PATH): the encode arguments the builder makes are
//! run with the real ffmpeg, with the .avs input swapped for an equivalent AVI, so
//! x264 / pixel format / audio / chapters / faststart / mapping are checked for real.
//! The box's Linux ffmpeg has no avisynth demuxer, so this part does NOT read a script.
//!
//! Part 2 (only when FFENC_WINE_FFMPEG points to a Windows ffmpeg.exe and `wine` is
//! available with AviSynth+ installed in that prefix): every script variant is written,
//! opened by the Windows ffmpeg through AviSynth+ under Wine, encoded with the exact
//! built command, and the output checked with ffprobe. The "signpost" is a plain
//! uncompressed AVI – a real DebugMode FrameServer signpost needs its codec and a
//! serving editor, which can't run here.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffenc::cmd::frameserve::*;
use ffenc::cmd::common::{AudioEncode, VideoEncode};
use serde_json::Value;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn probe(ffprobe: &Path, f: &Path) -> Value {
    let o = Command::new(ffprobe).args(["-v", "error", "-show_streams", "-show_format", "-show_chapters", "-of", "json"]).arg(f).output().unwrap();
    assert!(o.status.success(), "ffprobe {}: {}", f.display(), String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}
fn of_type<'a>(v: &'a Value, t: &str) -> Vec<&'a Value> {
    v["streams"].as_array().unwrap().iter().filter(|s| s["codec_type"] == t).collect()
}

fn enc(script_path: PathBuf, text: String, output: PathBuf) -> EncodeParams {
    EncodeParams {
        script_path,
        script_text: text,
        output,
        video: VideoEncode { crf: 30, preset: "ultrafast".into(), tune: None, ..Default::default() },
        audio: AudioEncode::default(),
        has_audio: true,
        pix_fmt: "yuv420p".into(),
        resize: None,
        bt709_tags: true,
        matrix: Matrix::Rec709,
        range_convert: false,
        chapters: None,
        faststart: true,
        picture: Default::default(),
    }
}

/// Linux path -> the Z:\ path Wine programs see.
fn z(p: &Path) -> String {
    format!("Z:{}", p.to_string_lossy().replace('/', "\\"))
}

/// Variants: (label, options tweak, pix_fmt expected, has audio, frames expected (of 120), fps expected)
fn variants() -> Vec<(&'static str, Box<dyn Fn(&mut ScriptOptions)>, &'static str, bool, u64, f64)> {
    vec![
        ("default", Box::new(|_o: &mut ScriptOptions| {}), "yuv420p", true, 120, 29.97),
        ("yv12 interlaced pt", Box::new(|o: &mut ScriptOptions| { o.color = Color::Yv12; o.interlaced = true; o.pixel_type = PixelType::Rgb32; }), "yuv420p", true, 120, 29.97),
        ("422 10-bit 16bit audio", Box::new(|o: &mut ScriptOptions| { o.color = Color::Yuv422; o.bits = BitDepth::Ten; o.audio = AudioMode::To16bit; o.matrix = Matrix::Pc709; }), "yuv422p10le", true, 120, 29.97),
        ("444 no audio trim fps", Box::new(|o: &mut ScriptOptions| { o.color = Color::Yuv444; o.audio = AudioMode::None; o.trim = Some((30, 89)); o.fps = Some((25, 1)); }), "yuv444p", false, 50, 25.0),
        ("rgb32", Box::new(|o: &mut ScriptOptions| { o.color = Color::Rgb32; o.matrix = Matrix::Rec601; }), "yuv420p", true, 120, 29.97),
        ("none", Box::new(|o: &mut ScriptOptions| { o.color = Color::None; o.trim = Some((100, 0)); }), "yuv420p", true, 20, 29.97),
    ]
}


/// (video length, audio length) in seconds from the packets: last pts + duration per stream type.
fn av_lengths(ffprobe: &Path, f: &Path) -> (f64, Option<f64>) {
    let o = Command::new(ffprobe).args(["-v", "error", "-show_entries", "packet=codec_type,pts_time,duration_time", "-of", "json"]).arg(f).output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let (mut vend, mut aend) = (0.0f64, None::<f64>);
    for p in v["packets"].as_array().unwrap() {
        let t: f64 = p["pts_time"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        let d: f64 = p["duration_time"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        match p["codec_type"].as_str().unwrap_or("") {
            "video" => vend = vend.max(t + d),
            "audio" => aend = Some(aend.unwrap_or(0.0).max(t + d)),
            _ => {}
        }
    }
    (vend, aend)
}

/// Frame-rate changes with audio: (label, ChangeFPS rate or None, output frame rate field).
const RATE_CASES: &[(&str, Option<(u32, u32)>, &str)] = &[
    ("rate default", None, ""),
    ("rate 23.976", Some((24000, 1001)), ""),
    ("rate 30", Some((30, 1)), ""),
    ("rate 60", Some((60, 1)), ""),
    ("field 23.976", None, "24000/1001"),
    ("field 30", None, "30"),
    ("field 60", None, "60"),
];

/// The audio and the video end together (within about one frame) whatever the frame rate.
fn assert_in_sync(label: &str, ffprobe: &Path, out: &Path) -> (f64, f64) {
    let (v, a) = av_lengths(ffprobe, out);
    let a = a.unwrap_or_else(|| panic!("{label}: no audio"));
    // one frame at the slowest rate here, plus AAC padding
    assert!((v - a).abs() < 1.0 / 23.976 + 0.03, "{label}: video {v:.3} s, audio {a:.3} s – out of sync");
    assert!((v - 4.004).abs() < 0.1, "{label}: length {v:.3} s changed (speed-up/slow-down)");
    (v, a)
}

#[test]
fn frameserve_commands_with_real_ffmpeg() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else {
        eprintln!("ffmpeg not on PATH – skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("ffenc frameserve ünï {}", ffenc::util::unique_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let avi = dir.join("sign post ünï.avi");
    let o = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30000/1001:duration=4", "-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
        .args(["-c:v", "rawvideo", "-pix_fmt", "bgr24", "-c:a", "pcm_s16le", "-ac", "2"])
        .arg(&avi)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // ---- part 1: encode arguments, with the AVI standing in for the script
    for (label, tweak, pix, audio, _, _) in variants() {
        let mut so = ScriptOptions::new(&avi);
        tweak(&mut so);
        let text = script(&so).unwrap();
        let mut pr = enc(dir.join("s.avs"), text, dir.join(format!("p1 {label}.mp4")));
        pr.pix_fmt = so.color.pix_fmt(so.bits);
        pr.has_audio = audio;
        pr.chapters = Some((dir.join("ch.ffmeta"), ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=One\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=1000\nEND=2000\ntitle=Two\n".into()));
        let b = build(&pr).unwrap();
        for (p, t) in &b.side_files {
            std::fs::write(p, t).unwrap();
        }
        let mut args = b.args.clone();
        let i = args.iter().position(|a| a == "-i").unwrap();
        args[i + 1] = avi.to_string_lossy().into_owned();
        let o = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&args).output().unwrap();
        assert!(o.status.success(), "{label}: ffmpeg {}\n{}", args.join(" "), String::from_utf8_lossy(&o.stderr));
        let v = probe(&ffprobe, &pr.output);
        let vs = of_type(&v, "video");
        assert_eq!(vs[0]["codec_name"], "h264", "{label}");
        assert_eq!(vs[0]["pix_fmt"], pix, "{label}");
        assert_eq!(of_type(&v, "audio").len(), audio as usize, "{label}");
        assert_eq!(v["chapters"].as_array().unwrap().len(), 2, "{label}");
    }

    // ---- part 1b: the output frame rate field (fps filter) keeps the audio in sync
    for (label, _, field) in RATE_CASES.iter().filter(|c| c.1.is_none()) {
        let so = ScriptOptions::new(&avi);
        let mut pr = enc(dir.join("s.avs"), script(&so).unwrap(), dir.join(format!("p1 {label}.mp4")));
        pr.video.output_fps = field.to_string();
        let mut args = build(&pr).unwrap().args;
        assert!(!args.iter().any(|a| a == "-r"), "{label}: never retime the input");
        let i = args.iter().position(|a| a == "-i").unwrap();
        args[i + 1] = avi.to_string_lossy().into_owned();
        let o = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&args).output().unwrap();
        assert!(o.status.success(), "{label}: {}", String::from_utf8_lossy(&o.stderr));
        let (v, a) = assert_in_sync(label, &ffprobe, &pr.output);
        eprintln!("field sync OK: {label}: video {v:.3} s, audio {a:.3} s");
    }

    // ---- part 2: real AviSynth+ through the Windows ffmpeg under Wine (optional)
    let Some(win_ffmpeg) = std::env::var_os("FFENC_WINE_FFMPEG").map(PathBuf::from) else {
        eprintln!("FFENC_WINE_FFMPEG not set – AviSynth part skipped (no avisynth demuxer on this ffmpeg)");
        std::fs::remove_dir_all(&dir).unwrap();
        return;
    };
    // FFENC_WINE: the wine loader to use (default "wine")
    let wine = std::env::var_os("FFENC_WINE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("wine"));
    for (label, tweak, pix, audio, frames, fps) in variants() {
        let mut so = ScriptOptions::new(z(&avi));
        tweak(&mut so);
        let text = script(&so).unwrap();
        let sp = dir.join(format!("{TEMP_PREFIX}{}.avs", label.replace(' ', "_")));
        let out = dir.join(format!("p2 {label}.mkv"));
        let mut pr = enc(PathBuf::from(z(&sp)), text, PathBuf::from(z(&out)));
        pr.pix_fmt = so.color.pix_fmt(so.bits);
        pr.has_audio = audio;
        pr.faststart = false;
        let b = build(&pr).unwrap();
        std::fs::write(&sp, &b.side_files[0].1).unwrap();
        let o = Command::new(&wine).arg(&win_ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&b.args).env("WINEDEBUG", "-all").output().unwrap();
        assert!(o.status.success() && out.is_file(), "{label}: wine ffmpeg {}\n{}\nscript:\n{}", b.args.join(" "), String::from_utf8_lossy(&o.stderr), b.side_files[0].1);
        let v = probe(&ffprobe, &out);
        let vs = of_type(&v, "video");
        assert_eq!(vs[0]["pix_fmt"], pix, "{label}");
        assert_eq!(of_type(&v, "audio").len(), audio as usize, "{label}");
        let d: f64 = v["format"]["duration"].as_str().unwrap().parse().unwrap();
        let want = frames as f64 / fps;
        assert!((d - want).abs() < 0.15, "{label}: duration {d}, expected about {want}");
        eprintln!("AviSynth+ under Wine OK: {label} -> {pix}, {d:.3} s");
    }
    // frame-rate changes in the script (ChangeFPS) and in the output field, with audio
    for (label, rate, field) in RATE_CASES {
        let mut so = ScriptOptions::new(z(&avi));
        so.fps = *rate;
        let text = script(&so).unwrap();
        assert!(!text.contains("AssumeFPS"), "{label}: AssumeFPS changes the speed of the video only");
        let sp = dir.join(format!("{TEMP_PREFIX}{}.avs", label.replace(' ', "_")));
        let out = dir.join(format!("p2 {label}.mp4"));
        let mut pr = enc(PathBuf::from(z(&sp)), text, PathBuf::from(z(&out)));
        pr.video.output_fps = field.to_string();
        let b = build(&pr).unwrap();
        std::fs::write(&sp, &b.side_files[0].1).unwrap();
        let o = Command::new(&wine).arg(&win_ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&b.args).env("WINEDEBUG", "-all").output().unwrap();
        assert!(o.status.success() && out.is_file(), "{label}: wine ffmpeg {}\n{}", b.args.join(" "), String::from_utf8_lossy(&o.stderr));
        let (v, a) = assert_in_sync(label, &ffprobe, &out);
        eprintln!("AviSynth+ under Wine sync OK: {label}: video {v:.3} s, audio {a:.3} s");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
