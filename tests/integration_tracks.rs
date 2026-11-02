//! "Audio & subtitles" tool end-to-end with the real ffmpeg: synthetic video with two audio
//! and two subtitle tracks (+ an attachment), external WAV / SRT (UTF-8 and Windows-1252) /
//! ASS / VTT files, audio that is one second off. Outputs are checked with ffprobe.
//! Skips (passes) when ffmpeg is not on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffenc::cmd::tracks::*;
use ffenc::cmd::common::CompressorPreset;
use serde_json::Value;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

struct Env {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    dir: PathBuf,
}

impl Env {
    fn gen(&self, args: &[&str]) {
        let o = Command::new(&self.ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).current_dir(&self.dir).output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }
    fn run(&self, args: &[String]) {
        let o = Command::new(&self.ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(args).output().unwrap();
        assert!(o.status.success(), "ffmpeg {}\n{}", args.join(" "), String::from_utf8_lossy(&o.stderr));
    }
    fn probe(&self, f: &Path) -> Value {
        let o = Command::new(&self.ffprobe).args(["-v", "error", "-show_streams", "-show_format", "-of", "json"]).arg(f).output().unwrap();
        assert!(o.status.success(), "ffprobe {}", f.display());
        serde_json::from_slice(&o.stdout).unwrap()
    }
    fn p(&self, n: &str) -> PathBuf {
        self.dir.join(n)
    }
}

fn of_type<'a>(v: &'a Value, t: &str) -> Vec<&'a Value> {
    v["streams"].as_array().unwrap().iter().filter(|s| s["codec_type"] == t).collect()
}
fn tag(s: &Value, k: &str) -> String {
    s["tags"][k].as_str().or_else(|| s["tags"][k.to_uppercase()].as_str()).unwrap_or("").to_string()
}
fn disp(s: &Value, k: &str) -> bool {
    s["disposition"][k].as_i64() == Some(1)
}
fn fdur(v: &Value) -> f64 {
    v["format"]["duration"].as_str().unwrap().parse().unwrap()
}
fn sdur(s: &Value) -> f64 {
    s["duration"].as_str().map(|d| d.parse().unwrap()).unwrap_or(-1.0)
}

fn setup() -> Option<Env> {
    let env = Env { ffmpeg: which("ffmpeg")?, ffprobe: which("ffprobe")?, dir: std::env::temp_dir().join(format!("ffenc tracks ünï {}", ffenc::util::unique_id())) };
    std::fs::create_dir_all(&env.dir).unwrap();
    std::fs::write(env.p("in_en.srt"), "1\n00:00:01,000 --> 00:00:03,000\nHello\n\n2\n00:00:05,000 --> 00:00:07,000\nWorld\n").unwrap();
    std::fs::write(env.p("in_de.srt"), "1\n00:00:02,000 --> 00:00:04,000\nHallo\n").unwrap();
    std::fs::write(env.p("font.txt"), "not a font").unwrap();
    // 10 s video, 2 audio (Game eng default, Mic ger), 2 subs (eng, ger forced), 1 attachment
    env.gen(&[
        "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=10", "-f", "lavfi", "-i", "sine=frequency=440:duration=10:sample_rate=48000",
        "-f", "lavfi", "-i", "sine=frequency=880:duration=10:sample_rate=48000", "-i", "in_en.srt", "-i", "in_de.srt",
        "-map", "0", "-map", "1", "-map", "2", "-map", "3", "-map", "4", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-c:s", "srt",
        "-metadata:s:a:0", "title=Game", "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Mic", "-metadata:s:a:1", "language=ger",
        "-metadata:s:s:0", "language=eng", "-metadata:s:s:1", "language=ger", "-disposition:s:1", "forced",
        "-attach", "font.txt", "-metadata:s:t", "mimetype=text/plain", "video.mkv",
    ]);
    // commentary 11 s (one second too long), short replacement 6 s, AAC m4a 10 s
    env.gen(&["-f", "lavfi", "-i", "sine=frequency=300:duration=11:sample_rate=48000", "-ac", "2", "commentary.wav"]);
    env.gen(&["-f", "lavfi", "-i", "sine=frequency=600:duration=6:sample_rate=48000", "-ac", "2", "short.wav"]);
    env.gen(&["-f", "lavfi", "-i", "sine=frequency=500:duration=10:sample_rate=48000", "-c:a", "aac", "second.m4a"]);
    std::fs::write(env.p("utf8.srt"), "1\n00:00:01,000 --> 00:00:02,000\nGrüße ☺\n").unwrap();
    std::fs::write(env.p("latin1.srt"), b"1\n00:00:01,000 --> 00:00:02,000\nGr\xfc\xdfe aus K\xf6ln\n".as_slice()).unwrap();
    std::fs::write(env.p("styled.ass"), "[Script Info]\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\i1}Styled{\\i0}\n").unwrap();
    std::fs::write(env.p("web.vtt"), "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nFrom VTT\n").unwrap();
    Some(env)
}

fn base(env: &Env, out: &str, audio: Vec<AudioTrack>, subs: Vec<SubTrack>) -> TracksParams {
    TracksParams {
        video: env.p("video.mkv"),
        video_duration: Some(10.0),
        audio,
        subs,
        output: env.p(out),
        aac_bitrate_k: 128,
        advanced: Default::default(),
        tolerance_s: 1.5,
        faststart: true,
    }
}

fn main_audio() -> Vec<AudioTrack> {
    vec![
        AudioTrack { stream: 0, codec: "aac".into(), title: "Game".into(), language: "eng".into(), default: true, ..Default::default() },
        AudioTrack { stream: 1, codec: "aac".into(), title: "Mic".into(), language: "ger".into(), ..Default::default() },
    ]
}
fn ext_audio(env: &Env, f: &str, codec: &str, dur: f64, title: &str) -> AudioTrack {
    AudioTrack { source: Source::File(env.p(f)), codec: codec.into(), duration: Some(dur), title: title.into(), language: "eng".into(), ..Default::default() }
}
fn ext_sub(env: &Env, f: &str, codec: &str, lang: &str, title: &str) -> SubTrack {
    SubTrack { source: Source::File(env.p(f)), codec: codec.into(), language: lang.into(), title: title.into(), ..Default::default() }
}

/// Text of a subtitle stream, via ffmpeg -> srt on stdout.
fn sub_text(env: &Env, f: &Path, idx: usize) -> String {
    let o = Command::new(&env.ffmpeg).args(["-v", "error", "-i"]).arg(f).args(["-map", &format!("0:s:{idx}"), "-c:s", "srt", "-f", "srt", "-"]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn audio_and_subtitles_tool() {
    let Some(env) = setup() else {
        eprintln!("ffmpeg not on PATH – skipped");
        return;
    };

    // 1. MKV: commentary first + default (1 s too long -> trimmed), originals kept, German sub removed,
    //    four external subtitle files (UTF-8, Windows-1252 auto-detected, ASS, VTT)
    let mut com = ext_audio(&env, "commentary.wav", "pcm_s16le", 11.0, "Commentary Track");
    com.default = true;
    let mut audio = vec![com];
    let mut ma = main_audio();
    ma[0].default = false;
    audio.extend(ma);
    let mut de = SubTrack { stream: 1, codec: "subrip".into(), language: "ger".into(), keep: false, ..Default::default() };
    de.forced = true;
    let mut latin = ext_sub(&env, "latin1.srt", "subrip", "ger", "Deutsch");
    latin.forced = true;
    let subs = vec![
        SubTrack { stream: 0, codec: "subrip".into(), language: "eng".into(), title: "English".into(), default: true, ..Default::default() },
        de,
        ext_sub(&env, "utf8.srt", "subrip", "en", "UTF-8 file"),
        latin,
        ext_sub(&env, "styled.ass", "ass", "eng", "Styled"),
        ext_sub(&env, "web.vtt", "webvtt", "fre", "VTT"),
    ];
    let pr = base(&env, "out1.mkv", audio, subs);
    assert_eq!(summary(&pr, 1), "Output will have: 1 video, 3 audio (Commentary Track, Game, Mic), 5 subtitles (English, UTF-8 file, Deutsch, Styled, VTT)");
    let b = build(&pr).unwrap();
    env.run(&b.args);
    let v = env.probe(&pr.output);
    let a = of_type(&v, "audio");
    assert_eq!(a.iter().map(|s| tag(s, "title")).collect::<Vec<_>>(), ["Commentary Track", "Game", "Mic"]);
    assert_eq!(a.iter().map(|s| tag(s, "language")).collect::<Vec<_>>(), ["eng", "eng", "ger"]);
    assert_eq!(a.iter().map(|s| disp(s, "default")).collect::<Vec<_>>(), [true, false, false]);
    assert_eq!(a[0]["codec_name"], "pcm_s16le", "WAV copied into MKV");
    let s = of_type(&v, "subtitle");
    assert_eq!(s.iter().map(|s| tag(s, "title")).collect::<Vec<_>>(), ["English", "UTF-8 file", "Deutsch", "Styled", "VTT"]);
    assert_eq!(s.iter().map(|s| tag(s, "language")).collect::<Vec<_>>(), ["eng", "eng", "ger", "eng", "fre"]);
    assert_eq!(s.iter().map(|s| disp(s, "default")).collect::<Vec<_>>(), [true, false, false, false, false]);
    assert_eq!(s.iter().map(|s| disp(s, "forced")).collect::<Vec<_>>(), [false, false, true, false, false]);
    assert_eq!(s.iter().map(|s| s["codec_name"].as_str().unwrap()).collect::<Vec<_>>(), ["subrip", "subrip", "subrip", "ass", "subrip"]);
    assert_eq!(of_type(&v, "attachment").len(), 1, "attachments kept in MKV");
    assert!((fdur(&v) - 10.0).abs() < 0.3, "trimmed to the video: {}", fdur(&v));
    assert!(sub_text(&env, &pr.output, 1).contains("Grüße ☺"));
    assert!(sub_text(&env, &pr.output, 2).contains("Grüße aus Köln"), "Windows-1252 decoded: {}", sub_text(&env, &pr.output, 2));

    // 2. MP4: replace all audio with a 6 s file padded with silence; subtitles -> mov_text; titles via handler_name
    let mut orig = main_audio();
    for t in &mut orig {
        t.keep = false;
    }
    let mut new = ext_audio(&env, "short.wav", "pcm_s16le", 6.0, "New mix");
    new.fit = Fit::Pad;
    new.default = true;
    let mut audio = vec![new];
    audio.extend(orig);
    let subs = vec![
        SubTrack { stream: 0, codec: "subrip".into(), language: "eng".into(), title: "English".into(), ..Default::default() },
        ext_sub(&env, "styled.ass", "ass", "eng", "Styled"),
    ];
    let pr = base(&env, "out2.mp4", audio.clone(), subs);
    let b = build(&pr).unwrap();
    assert!(b.warnings.iter().any(|w| w.contains("styled ASS")));
    env.run(&b.args);
    let v = env.probe(&pr.output);
    let a = of_type(&v, "audio");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["codec_name"], "aac");
    assert_eq!(tag(a[0], "handler_name"), "New mix");
    assert!((sdur(a[0]) - 10.0).abs() < 0.3, "padded to 10 s: {}", sdur(a[0]));
    let s = of_type(&v, "subtitle");
    assert_eq!(s.iter().map(|s| s["codec_name"].as_str().unwrap()).collect::<Vec<_>>(), ["mov_text", "mov_text"]);
    assert_eq!(s.iter().map(|s| tag(s, "handler_name")).collect::<Vec<_>>(), ["English", "Styled"]);
    assert_eq!(s.iter().map(|s| tag(s, "language")).collect::<Vec<_>>(), ["eng", "eng"]);

    // 3. same but "leave as is": the video is NOT cut short (no -shortest), audio just ends early
    audio[0].fit = Fit::Auto;
    let pr = base(&env, "out3.mp4", audio, vec![]);
    env.run(&build(&pr).unwrap().args);
    let v = env.probe(&pr.output);
    assert!((sdur(of_type(&v, "video")[0]) - 10.0).abs() < 0.2, "video kept its length");
    assert!((sdur(of_type(&v, "audio")[0]) - 6.0).abs() < 0.3);

    // 4. second language track (AAC) copied into MP4 with a 1 s delay; commentary with gain,
    //    compressor and 0.5 s delay mixed into the main track
    let mut second = ext_audio(&env, "second.m4a", "aac", 10.0, "Second language");
    second.delay_ms = 1000;
    second.language = "spa".into();
    second.fit = Fit::KeepFull;
    let mut com = ext_audio(&env, "commentary.wav", "pcm_s16le", 11.0, "Commentary");
    com.delay_ms = 500;
    com.gain_db = 3.0;
    com.compressor = CompressorPreset::Light;
    com.mix_with_main = true;
    let mut audio = main_audio();
    audio.push(second);
    audio.push(com);
    let pr = base(&env, "out4.mp4", audio, vec![]);
    let b = build(&pr).unwrap();
    assert!(b.args.join(" ").contains("-itsoffset 1 -i"), "{}", b.args.join(" "));
    env.run(&b.args);
    let v = env.probe(&pr.output);
    let a = of_type(&v, "audio");
    assert_eq!(a.iter().map(|s| tag(s, "handler_name")).collect::<Vec<_>>(), ["Game", "Mic", "Second language", "Commentary", "Game + Commentary"]);
    assert_eq!(a.iter().map(|s| tag(s, "language")).collect::<Vec<_>>(), ["eng", "ger", "spa", "eng", "eng"]);
    assert_eq!(a[2]["codec_name"], "aac");
    let st: f64 = a[2]["start_time"].as_str().unwrap().parse().unwrap();
    assert!((st - 1.0).abs() < 0.1, "delayed by 1 s: start_time {st}");
    assert!((sdur(a[3]) - 10.0).abs() < 0.3, "commentary: 0.5 s silence + trimmed to the video: {}", sdur(a[3]));
    assert!((sdur(a[4]) - 10.0).abs() < 0.3, "mix follows the main track: {}", sdur(a[4]));
    assert!((fdur(&v) - 11.0).abs() < 0.3, "kept-full second track runs 1 s longer: {}", fdur(&v));

    // 5. extract every audio format + subtitles
    for (f, codec) in [(AudioFormat::Wav, "pcm_s16le"), (AudioFormat::Flac, "flac"), (AudioFormat::Mp3, "mp3"), (AudioFormat::M4a, "aac"), (AudioFormat::Opus, "opus"), (AudioFormat::Original, "aac")] {
        let name = extract_name(&env.p("video.mkv"), "audio", 2, "ger", "Mic", audio_ext(f, "aac"));
        let out = env.p(&name);
        env.run(&extract_audio(&env.p("video.mkv"), 1, "aac", f, 128, &out).args);
        let v = env.probe(&out);
        assert_eq!(v["streams"].as_array().unwrap().len(), 1, "{name}");
        assert_eq!(of_type(&v, "audio")[0]["codec_name"], codec, "{name}");
    }
    assert!(env.p("video - audio 2 (ger, Mic).m4a").is_file());
    for (f, codec) in [(SubFormat::Srt, "subrip"), (SubFormat::Ass, "ass"), (SubFormat::Original, "subrip")] {
        let (ext, _) = sub_target(f, "subrip").unwrap();
        let out = env.p(&format!("sub.{:?}.{ext}", f));
        env.run(&extract_sub(&env.p("video.mkv"), 1, "subrip", f, &out).unwrap().args);
        assert_eq!(of_type(&env.probe(&out), "subtitle")[0]["codec_name"], codec);
    }
    // MP4's own text subtitles extract to SRT
    let out = env.p("from_mp4.srt");
    env.run(&extract_sub(&env.p("out2.mp4"), 0, "mov_text", SubFormat::Original, &out).unwrap().args);
    assert!(std::fs::read_to_string(&out).unwrap().contains("Hello"));

    std::fs::remove_dir_all(&env.dir).unwrap();
}
