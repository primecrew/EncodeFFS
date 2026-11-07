//! "Join / stitch files": concatenate several inputs into one output.
//!
//! * **Lossless** – concat demuxer: `-f concat -safe 0 -i <list> -c copy`. The list
//!   file uses FFmpeg's concat syntax (`ffconcat version 1.0`, then `file '<path>'`
//!   per input; a `'` inside the path is written `'\''`; inside single quotes every
//!   other character, including Windows backslashes, is literal). All inputs must
//!   share codecs and parameters – [`compat_issues`] reports differences.
//! * **Re-encode** – concat filter. Every segment is normalised to the first file:
//!   scale + pad to its size, `setsar=1`, its frame rate, yuv420p; audio resampled to
//!   its rate and channel layout (mono or stereo). Inputs without audio get silence,
//!   inputs without video get black frames. Encoded with the preset's x264/AAC settings.
//!
//! Optional chapters: one per input (titled with the file name), start/end from the
//! probed durations, written with the FFMETADATA1 chapter writer.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::chapters::{self, Chapter};
use super::common::*;
use super::{p, ArgVec, BuiltCommand};
use crate::ffmpeg::probe::MediaInfo;
use crate::util::fmt_num;

pub const LIST_PREFIX: &str = "ffenc_concat_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum JoinMethod {
    /// Concat demuxer, stream copy.
    #[default]
    Copy,
    /// Concat filter, re-encode.
    Reencode,
}
impl JoinMethod {
    pub fn label(self) -> &'static str {
        match self {
            JoinMethod::Copy => "Fast, lossless (concat demuxer, stream copy)",
            JoinMethod::Reencode => "Re-encode (concat filter, normalises size/fps/audio)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VideoParams {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub pix_fmt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioParams {
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

/// One input as far as joining is concerned.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JoinInput {
    pub path: PathBuf,
    pub duration_s: f64,
    pub video: Option<VideoParams>,
    pub audio: Option<AudioParams>,
    pub audio_tracks: usize,
}

impl JoinInput {
    pub fn from_info(path: &Path, i: &MediaInfo) -> Result<Self, String> {
        // the concat demuxer advances by the container duration, so prefer it
        let duration_s = i.format_duration.filter(|d| *d > 0.0).or_else(|| i.any_duration()).ok_or_else(|| format!("{}: duration unknown", path.display()))?;
        let video = i.video().map(|v| VideoParams { codec: v.codec_name.clone(), width: v.width.unwrap_or(0), height: v.height.unwrap_or(0), fps: v.fps, pix_fmt: v.pix_fmt.clone() });
        let a = i.of_type("audio").next();
        let audio = a.map(|a| AudioParams { codec: a.codec_name.clone(), sample_rate: a.sample_rate, channels: a.channels });
        Ok(JoinInput { path: path.to_path_buf(), duration_s, video, audio, audio_tracks: i.of_type("audio").count() })
    }
    pub fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// Differences that break a lossless join (compared with the first file).
pub fn compat_issues(inputs: &[JoinInput]) -> Vec<String> {
    let Some(first) = inputs.first() else { return vec![] };
    let mut out = vec![];
    for f in &inputs[1..] {
        let n = f.name();
        let mut d = |what: &str, a: String, b: String| {
            if a != b {
                out.push(format!("{n}: {what} {b} differs from {a} (first file)"));
            }
        };
        match (&first.video, &f.video) {
            (Some(a), Some(b)) => {
                d("video codec", a.codec.clone(), b.codec.clone());
                d("resolution", format!("{}x{}", a.width, a.height), format!("{}x{}", b.width, b.height));
                let fps = |x: Option<f64>| x.map(|v| fmt_num((v * 1000.0).round() / 1000.0)).unwrap_or_else(|| "?".into());
                d("frame rate", fps(a.fps), fps(b.fps));
                d("pixel format", a.pix_fmt.clone().unwrap_or_default(), b.pix_fmt.clone().unwrap_or_default());
            }
            (a, b) => d("video", if a.is_some() { "present" } else { "none" }.into(), if b.is_some() { "present" } else { "none" }.into()),
        }
        match (&first.audio, &f.audio) {
            (Some(a), Some(b)) => {
                d("audio codec", a.codec.clone(), b.codec.clone());
                d("sample rate", a.sample_rate.map(|r| format!("{r} Hz")).unwrap_or_default(), b.sample_rate.map(|r| format!("{r} Hz")).unwrap_or_default());
                d("audio channels", a.channels.map(|c| c.to_string()).unwrap_or_default(), b.channels.map(|c| c.to_string()).unwrap_or_default());
            }
            (a, b) => d("audio", if a.is_some() { "present" } else { "none" }.into(), if b.is_some() { "present" } else { "none" }.into()),
        }
        d("audio track count", first.audio_tracks.to_string(), f.audio_tracks.to_string());
    }
    out
}

/// Escape a path for a concat-list `file '…'` line.
pub fn concat_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// The concat demuxer list file.
pub fn concat_list(paths: &[PathBuf]) -> String {
    let mut s = String::from("ffconcat version 1.0\n");
    for p in paths {
        s.push_str("file ");
        s.push_str(&concat_quote(p));
        s.push('\n');
    }
    s
}

/// One chapter per input, cumulative from the probed durations (ms, rounded once
/// on the running total so rounding never accumulates).
pub fn file_chapters(inputs: &[JoinInput]) -> Vec<Chapter> {
    let mut out = vec![];
    let mut acc = 0.0f64;
    for f in inputs {
        let start = (acc * 1000.0).round() as i64;
        acc += f.duration_s;
        let end = (acc * 1000.0).round() as i64;
        let title = f.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| format!("Part {}", out.len() + 1));
        out.push(Chapter { start_ms: start, end_ms: end, title });
    }
    out
}

pub fn total_duration(inputs: &[JoinInput]) -> f64 {
    inputs.iter().map(|f| f.duration_s).sum()
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinParams {
    pub inputs: Vec<JoinInput>,
    pub output: PathBuf,
    pub method: JoinMethod,
    pub chapters_per_file: bool,
    pub video: VideoEncode,
    pub audio: AudioEncode,
    pub faststart: bool,
    /// Temp concat list (lossless method).
    pub list_file: PathBuf,
    /// Temp chapter file (when `chapters_per_file`).
    pub chapter_file: PathBuf,
}

impl JoinParams {
    pub fn output_duration(&self) -> f64 {
        total_duration(&self.inputs)
    }
}

pub fn temp_list_path() -> PathBuf {
    std::env::temp_dir().join(format!("{LIST_PREFIX}{}.txt", crate::util::unique_id()))
}

fn fps_arg(f: f64) -> String {
    // common NTSC rates as exact fractions
    for (n, d) in [(24000u32, 1001u32), (30000, 1001), (60000, 1001)] {
        if (f - n as f64 / d as f64).abs() < 0.002 {
            return format!("{n}/{d}");
        }
    }
    fmt_num((f * 1000.0).round() / 1000.0)
}

pub fn build(pr: &JoinParams) -> Result<BuiltCommand, String> {
    if pr.inputs.len() < 2 {
        return Err("Add at least two files to join.".into());
    }
    pr.audio.validate()?;
    let ext = crate::util::ext_lower(&pr.output);
    let mkv = matches!(ext.as_str(), "mkv" | "mka" | "webm");
    let mp4 = super::reencode::is_mp4_like(&pr.output);
    for f in &pr.inputs {
        if crate::util::same_path(&f.path, &pr.output) {
            return Err(format!("The output would overwrite an input: {}", f.path.display()));
        }
    }
    let mut cmd = BuiltCommand::default();
    let mut a: Vec<String> = vec!["-y".into()];
    let chapter_input;
    match pr.method {
        JoinMethod::Copy => {
            a.push_all(["-f", "concat", "-safe", "0", "-i"]);
            a.push(p(&pr.list_file));
            cmd.side_files.push((pr.list_file.clone(), concat_list(&pr.inputs.iter().map(|f| f.path.clone()).collect::<Vec<_>>())));
            chapter_input = 1;
        }
        JoinMethod::Reencode => {
            for f in &pr.inputs {
                a.push("-i".into());
                a.push(p(&f.path));
            }
            chapter_input = pr.inputs.len();
        }
    }
    if pr.chapters_per_file {
        let list = file_chapters(&pr.inputs);
        chapters::validate(&list, total_duration(&pr.inputs))?;
        a.push_all(["-f", "ffmetadata", "-i"]);
        a.push(p(&pr.chapter_file));
        cmd.side_files.push((pr.chapter_file.clone(), chapters::ffmetadata(&list)));
    }
    match pr.method {
        JoinMethod::Copy => {
            a.push_all(["-map", "0:v?", "-map", "0:a?"]);
            if mkv {
                a.push_all(["-map", "0:s?"]);
            }
            a.push_all(["-c", "copy"]);
            let issues = compat_issues(&pr.inputs);
            if !issues.is_empty() {
                cmd.warnings.push(format!("The files differ, a lossless join may fail or play badly – consider the re-encode method:\n{}", issues.join("\n")));
            }
        }
        JoinMethod::Reencode => {
            let has_v = pr.inputs.iter().any(|f| f.video.is_some());
            let has_a = pr.inputs.iter().any(|f| f.audio.is_some());
            let fv = pr.inputs.iter().find_map(|f| f.video.clone());
            let fa = pr.inputs.iter().find_map(|f| f.audio.clone());
            let (w, h) = fv.as_ref().map(|v| (v.width / 2 * 2, v.height / 2 * 2)).unwrap_or((0, 0));
            if has_v && (w == 0 || h == 0) {
                return Err("The first video has no usable resolution.".into());
            }
            // every file is normalised to one rate: the typed output frame rate, else the first file's
            let fps = match parse_output_fps(&pr.video.output_fps)? {
                Some(r) => r,
                None => fps_arg(fv.as_ref().and_then(|v| v.fps).unwrap_or(30.0)),
            };
            let rate = fa.as_ref().and_then(|a| a.sample_rate).unwrap_or(48000);
            let layout = if fa.as_ref().and_then(|a| a.channels) == Some(1) { "mono" } else { "stereo" };
            let mut g: Vec<String> = vec![];
            let mut pads = String::new();
            for (i, f) in pr.inputs.iter().enumerate() {
                let d = fmt_num(f.duration_s);
                if has_v {
                    if f.video.is_some() {
                        g.push(format!("[{i}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps},format=yuv420p[v{i}]"));
                    } else {
                        g.push(format!("color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p,setsar=1[v{i}]"));
                    }
                    pads.push_str(&format!("[v{i}]"));
                }
                if has_a {
                    if f.audio.is_some() {
                        g.push(format!("[{i}:a:0]aresample={rate},aformat=sample_fmts=fltp:sample_rates={rate}:channel_layouts={layout}[a{i}]"));
                    } else {
                        g.push(format!("anullsrc=r={rate}:cl={layout},atrim=duration={d}[a{i}]"));
                    }
                    pads.push_str(&format!("[a{i}]"));
                }
            }
            let (vo, ao) = (if has_v { "[vc]" } else { "" }, if has_a { "[ac]" } else { "" });
            g.push(format!("{pads}concat=n={}:v={}:a={}{vo}{ao}", pr.inputs.len(), has_v as u8, has_a as u8));
            let mut amap = "[ac]".to_string();
            if has_a {
                if let Some(f) = pr.audio.filter_chain() {
                    g.push(format!("[ac]{f}[aout]"));
                    amap = "[aout]".into();
                }
            }
            a.push("-filter_complex".into());
            a.push(g.join(";"));
            if has_v {
                a.push_all(["-map", "[vc]"]);
                cmd.warnings.extend(super::codecs::check_video(&pr.video, &pr.output)?);
                // the output rate is already in the graph above
                a.extend(super::codecs::video_args(&VideoEncode { output_fps: String::new(), ..pr.video.clone() }, None, false, &pr.output));
            }
            if has_a {
                a.push("-map".into());
                a.push(amap);
                if pr.audio.codec == super::codecs::AudioCodec::Copy {
                    return Err("Joining with re-encode mixes the audio through the concat filter, so it can't be copied. Choose AAC or AC3.".into());
                }
                cmd.warnings.extend(super::codecs::check_audio(&pr.audio, &pr.output)?);
                a.extend(super::codecs::audio_args(&pr.audio));
            }
            if has_v {
                cmd.warnings.push(format!("Re-encode join: every file is scaled/padded to {w}x{h} at {fps} fps{}; subtitles are not carried over.", if has_a { format!(", audio {rate} Hz {layout}") } else { String::new() }));
            }
        }
    }
    a.push("-map_chapters".into());
    a.push(if pr.chapters_per_file { chapter_input.to_string() } else { "-1".into() });
    if mp4 && pr.faststart {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    cmd.args = a;
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inp(name: &str, d: f64, w: u32, fps: f64, rate: u32, audio: bool) -> JoinInput {
        JoinInput {
            path: PathBuf::from(format!("/v/{name}")),
            duration_s: d,
            video: Some(VideoParams { codec: "h264".into(), width: w, height: w * 9 / 16, fps: Some(fps), pix_fmt: Some("yuv420p".into()) }),
            audio: audio.then(|| AudioParams { codec: "aac".into(), sample_rate: Some(rate), channels: Some(2) }),
            audio_tracks: audio as usize,
        }
    }

    fn params(inputs: Vec<JoinInput>, method: JoinMethod, ch: bool, out: &str) -> JoinParams {
        JoinParams {
            inputs,
            output: PathBuf::from(out),
            method,
            chapters_per_file: ch,
            video: VideoEncode::default(),
            audio: AudioEncode::default(),
            faststart: true,
            list_file: PathBuf::from("/tmp/ffenc_concat_1.txt"),
            chapter_file: PathBuf::from("/tmp/ffenc_chapters_1.ffmeta"),
        }
    }

    #[test]
    fn list_escaping() {
        assert_eq!(concat_quote(Path::new(r"C:\Videos\it's me\a b.mkv")), r"'C:\Videos\it'\''s me\a b.mkv'");
        assert_eq!(concat_quote(Path::new("/x/''.mkv")), r"'/x/'\'''\''.mkv'");
        let l = concat_list(&[PathBuf::from(r"D:\clips\ünï 視頻.mp4"), PathBuf::from("/a/b c.mp4")]);
        assert_eq!(l, "ffconcat version 1.0\nfile 'D:\\clips\\ünï 視頻.mp4'\nfile '/a/b c.mp4'\n");
    }

    #[test]
    fn chapter_offsets_cumulative() {
        let v = vec![inp("Part 1.mkv", 10.0004, 1920, 30.0, 48000, true), inp("Part 2.mkv", 5.0004, 1920, 30.0, 48000, true), inp("it's 3.mkv", 2.0004, 1920, 30.0, 48000, true)];
        let c = file_chapters(&v);
        assert_eq!(c.iter().map(|c| (c.start_ms, c.end_ms)).collect::<Vec<_>>(), [(0, 10000), (10000, 15001), (15001, 17001)], "rounded on the running total");
        assert_eq!(c[2].title, "it's 3");
        assert!(chapters::validate(&c, total_duration(&v)).is_ok());
        assert!((total_duration(&v) - 17.0012).abs() < 1e-9);
    }

    #[test]
    fn copy_command_and_compat() {
        let v = vec![inp("a.mp4", 3.0, 1920, 30.0, 48000, true), inp("b.mp4", 4.0, 1280, 29.97, 44100, true), inp("c.mp4", 5.0, 1920, 30.0, 48000, false)];
        let issues = compat_issues(&v);
        assert!(issues.iter().any(|i| i.starts_with("b.mp4: resolution 1280x720 differs from 1920x1080")), "{issues:?}");
        assert!(issues.iter().any(|i| i.contains("frame rate 29.97 differs from 30")));
        assert!(issues.iter().any(|i| i.contains("sample rate 44100 Hz")));
        assert!(issues.iter().any(|i| i.starts_with("c.mp4: audio none differs from present")));
        assert!(compat_issues(&[v[0].clone(), v[0].clone()]).is_empty());
        let b = build(&params(v.clone(), JoinMethod::Copy, true, "/o/out.mp4")).unwrap();
        assert_eq!(
            b.args.join(" "),
            "-y -f concat -safe 0 -i /tmp/ffenc_concat_1.txt -f ffmetadata -i /tmp/ffenc_chapters_1.ffmeta -map 0:v? -map 0:a? -c copy -map_chapters 1 -movflags +faststart /o/out.mp4"
        );
        assert_eq!(b.side_files.len(), 2);
        assert!(b.side_files[0].1.contains("file '/v/b.mp4'"));
        assert!(b.side_files[1].1.contains("START=7000\nEND=12000\ntitle=c"));
        assert!(b.warnings[0].contains("re-encode"));
        let b = build(&params(v[..2].to_vec(), JoinMethod::Copy, false, "/o/out.mkv")).unwrap();
        assert!(b.args.join(" ").contains("-map 0:s? -c copy -map_chapters -1 /o/out.mkv"), "{:?}", b.args);
        assert_eq!(b.side_files.len(), 1);
        assert!(build(&params(v[..1].to_vec(), JoinMethod::Copy, false, "/o/x.mkv")).is_err());
        assert!(build(&params(v.clone(), JoinMethod::Copy, false, "/v/a.mp4")).unwrap_err().contains("overwrite"));
    }

    #[test]
    fn reencode_graph() {
        let mut v = vec![inp("a.mp4", 3.0, 1921, 29.97003, 44100, true), inp("b.mp4", 4.0, 1280, 25.0, 48000, false)];
        v[0].video.as_mut().unwrap().height = 1081;
        let mut pr = params(v, JoinMethod::Reencode, true, "/o/j.mkv");
        pr.audio.gain_db = 3.0;
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.starts_with("-y -i /v/a.mp4 -i /v/b.mp4 -f ffmetadata -i /tmp/ffenc_chapters_1.ffmeta -filter_complex "), "{s}");
        let g = &b.args[b.args.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert_eq!(
            g,
            "[0:v:0]scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30000/1001,format=yuv420p[v0];\
[0:a:0]aresample=44100,aformat=sample_fmts=fltp:sample_rates=44100:channel_layouts=stereo[a0];\
[1:v:0]scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30000/1001,format=yuv420p[v1];\
anullsrc=r=44100:cl=stereo,atrim=duration=4[a1];\
[v0][a0][v1][a1]concat=n=2:v=1:a=1[vc][ac];[ac]volume=3dB[aout]"
        );
        assert!(s.contains("-map [vc] -c:v libx264 -crf 22 -preset slow -pix_fmt yuv420p -map [aout] -c:a aac -b:a 192k -map_chapters 2 /o/j.mkv"), "{s}");
        assert!(b.side_files.iter().all(|f| !f.0.to_string_lossy().contains("concat")), "no list file for the filter method");
        // a typed output frame rate replaces the first file's rate in the graph (no extra -vf / -r)
        pr.video.output_fps = "5".into();
        let b5 = build(&pr).unwrap();
        let g5 = &b5.args[b5.args.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert_eq!(g5.matches("fps=5,").count(), 2, "{g5}");
        assert!(!g5.contains("30000/1001") && !b5.args.iter().any(|a| a == "-vf" || a == "-r"));
        pr.video.output_fps = "-5".into();
        assert!(build(&pr).is_err(), "invalid rate rejected");
    }
}
