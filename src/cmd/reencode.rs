//! Mode 1: re-encode a single video (also used per file by mode 7 batch).
//!
//! Argument layout (fast input seeking, chapters relative to the trimmed output):
//! `-y [-ss START] -i INPUT [-f ffmetadata -i CHAPTERS] [-t OUTDUR] <video> <audio>
//!  [-filter:a CHAIN] -map 0 [-map -0:N ...] [-map_chapters 1] -c:s .. -c:d copy [-c:t copy]
//!  [-movflags +faststart] OUTPUT`

use std::path::PathBuf;

use super::common::*;
use super::{p, ArgVec, BuiltCommand};
use crate::util::{ext_lower, fmt_num};

/// Minimal description of an input stream that affects mapping into MP4.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamRef {
    /// Absolute stream index in the input file.
    pub index: u32,
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReencodeParams {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Full input duration in seconds (ffprobe or the user's manual override).
    pub duration_s: f64,
    pub trim_start: f64,
    pub trim_end: f64,
    pub passthrough: Passthrough,
    pub video: VideoEncode,
    pub audio: AudioEncode,
    /// Full -> limited BT.709 conversion + tagging (only when video is re-encoded).
    pub range_convert: bool,
    /// Final (even) dimensions when resizing.
    pub resize: Option<(u32, u32)>,
    /// Apply bwdif (caller passes `interlaced && user wants it`).
    pub deinterlace: bool,
    /// (path, FFMETADATA contents) of the chapter file, if any.
    pub chapters: Option<(PathBuf, String)>,
    pub faststart: bool,
    /// Subtitle streams in the input (needed for MP4 compatibility handling).
    pub subtitle_streams: Vec<StreamRef>,
    /// Input has attachment streams (fonts in MKV) – not allowed in MP4.
    pub has_attachments: bool,
}

impl ReencodeParams {
    /// Duration of the trimmed output.
    pub fn output_duration(&self) -> f64 {
        self.duration_s - self.trim_start - self.trim_end
    }
}

/// True for containers handled by ffmpeg's mov/mp4 muxer.
pub fn is_mp4_like(path: &std::path::Path) -> bool {
    matches!(ext_lower(path).as_str(), "mp4" | "m4v" | "mov")
}

/// Default output extension per the script: .mkv if input .mkv else .mp4.
pub fn default_output_ext(input: &std::path::Path) -> &'static str {
    if ext_lower(input) == "mkv" {
        "mkv"
    } else {
        "mp4"
    }
}

pub fn build(pr: &ReencodeParams) -> Result<BuiltCommand, String> {
    if pr.trim_start < 0.0 || pr.trim_end < 0.0 {
        return Err("Negative trim values".into());
    }
    if pr.duration_s <= 0.0 {
        return Err(format!("Invalid duration: {} seconds", pr.duration_s));
    }
    let total_trim = pr.trim_start + pr.trim_end;
    if total_trim >= pr.duration_s {
        return Err(format!(
            "Trim duration ({}) exceeds video duration ({})",
            fmt_num(total_trim),
            fmt_num(pr.duration_s)
        ));
    }
    if pr.passthrough.encodes_audio() {
        pr.audio.validate()?;
    }
    let out_dur = pr.output_duration();
    let mp4 = is_mp4_like(&pr.output);
    let mut warnings = Vec::new();
    let mut side_files = Vec::new();

    let mut a: Vec<String> = vec!["-y".into()];
    if pr.trim_start > 0.0 {
        a.push_all(["-ss".to_string(), fmt_num(pr.trim_start)]);
    }
    a.push_all(["-i".to_string(), p(&pr.input)]);
    if let Some((path, text)) = &pr.chapters {
        a.push_all(["-f", "ffmetadata", "-i"]);
        a.push(p(path));
        side_files.push((path.clone(), text.clone()));
    }
    if pr.trim_start > 0.0 || pr.trim_end > 0.0 {
        a.push_all(["-t".to_string(), fmt_num(out_dur)]);
    }

    // ---- video
    if pr.passthrough.encodes_video() {
        let resize = pr.resize.map(|(w, h)| format!("scale={w}:{h}"));
        let filter = join_filters(&[
            pr.deinterlace.then(|| DEINTERLACE_FILTER.to_string()),
            resize,
            pr.range_convert.then(|| RANGE_FILTER.to_string()),
        ]);
        warnings.extend(super::codecs::check_video(&pr.video, &pr.output)?);
        a.extend(super::codecs::video_args(&pr.video, filter.as_deref(), pr.range_convert, &pr.output));
    } else {
        a.push_all(["-c:v", "copy"]);
        if pr.deinterlace {
            warnings.push("Video passthrough selected: no deinterlacing filter will be applied.".into());
        }
        if pr.resize.is_some() || pr.range_convert {
            warnings.push("Video passthrough selected: resize / range conversion are ignored.".into());
        }
        if pr.trim_start > 0.0 {
            warnings.push("Trimming with video stream copy cuts on keyframes, so the start may be slightly earlier than requested.".into());
        }
    }

    // ---- audio
    if pr.passthrough.encodes_audio() {
        warnings.extend(super::codecs::check_audio(&pr.audio, &pr.output)?);
        a.extend(super::codecs::audio_args(&pr.audio));
        // A plain -filter:a creates one simple filtergraph per output audio stream,
        // so every audio track gets the gain/compressor while keeping its order,
        // language/title tags and dispositions (no filter_complex needed).
        if let Some(chain) = pr.audio.filter_chain().filter(|_| pr.audio.codec != super::codecs::AudioCodec::Copy) {
            a.push_all(["-filter:a".to_string(), chain]);
        }
    } else {
        a.push_all(["-c:a", "copy"]);
    }

    // ---- mapping
    a.push_all(["-map", "0"]);
    let mut sub_codec = "copy";
    if mp4 {
        let mut kept_subs = 0;
        for s in &pr.subtitle_streams {
            if mp4_sub_compatible(&s.codec) {
                kept_subs += 1;
            } else {
                a.push_all(["-map".to_string(), format!("-0:{}", s.index)]);
                warnings.push(format!(
                    "Subtitle stream {} ({}) cannot be stored in MP4 and is skipped. Use MKV output to keep it.",
                    s.index, s.codec
                ));
            }
        }
        if kept_subs > 0 {
            sub_codec = "mov_text";
        }
        if pr.has_attachments {
            a.push_all(["-map", "-0:t"]);
            warnings.push("Attachments (e.g. MKV fonts) cannot be stored in MP4 and are skipped.".into());
        }
    }
    if pr.chapters.is_some() {
        a.push_all(["-map_chapters", "1"]);
    }
    a.push_all(["-c:s", sub_codec, "-c:d", "copy"]);
    if !mp4 {
        a.push_all(["-c:t", "copy"]);
    }
    if pr.faststart && mp4 {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, side_files, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ReencodeParams {
        ReencodeParams {
            input: "/in/My Video.mkv".into(),
            output: "/out/My Video_encoded.mkv".into(),
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

    #[test]
    fn plain_reencode() {
        let b = build(&base()).unwrap();
        assert_eq!(
            b.args,
            vec!["-y", "-i", "/in/My Video.mkv", "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p",
                 "-c:a", "aac", "-b:a", "192k", "-map", "0", "-c:s", "copy", "-c:d", "copy", "-c:t", "copy",
                 "/out/My Video_encoded.mkv"]
        );
    }

    #[test]
    fn trim_uses_input_seek_and_output_duration() {
        let mut pr = base();
        pr.trim_start = 10.5;
        pr.trim_end = 4.5;
        let b = build(&pr).unwrap();
        assert_eq!(&b.args[0..5], &["-y", "-ss", "10.5", "-i", "/in/My Video.mkv"]);
        let t = b.args.iter().position(|x| x == "-t").unwrap();
        assert_eq!(b.args[t + 1], "85");
        // only end trim: no -ss but -t
        let mut pr = base();
        pr.trim_end = 20.0;
        let b = build(&pr).unwrap();
        assert!(!b.args.contains(&"-ss".to_string()));
        assert!(b.args.windows(2).any(|w| w == ["-t", "80"]));
    }

    #[test]
    fn trim_too_long() {
        let mut pr = base();
        pr.trim_start = 60.0;
        pr.trim_end = 40.0;
        assert!(build(&pr).is_err());
        pr.trim_end = -1.0;
        assert!(build(&pr).is_err());
    }

    #[test]
    fn passthrough_variants() {
        for (pt, v, a) in [
            (Passthrough::None, "libx264", "aac"),
            (Passthrough::Audio, "libx264", "copy"),
            (Passthrough::Video, "copy", "aac"),
            (Passthrough::Both, "copy", "copy"),
        ] {
            let mut pr = base();
            pr.passthrough = pt;
            pr.range_convert = true;
            pr.audio.gain_db = 2.0;
            let b = build(&pr).unwrap();
            let cv = b.args.iter().position(|x| x == "-c:v").unwrap();
            let ca = b.args.iter().position(|x| x == "-c:a").unwrap();
            assert_eq!(b.args[cv + 1], v, "{pt:?}");
            assert_eq!(b.args[ca + 1], a, "{pt:?}");
            // range conversion + full tags whenever video is encoded (consistent across variants)
            assert_eq!(b.args.contains(&"-colorspace".to_string()), pt.encodes_video(), "{pt:?}");
            assert_eq!(b.args.contains(&RANGE_FILTER.to_string()), pt.encodes_video(), "{pt:?}");
            // gain filter only when audio is encoded
            assert_eq!(b.args.contains(&"-filter:a".to_string()), pt.encodes_audio(), "{pt:?}");
        }
    }

    #[test]
    fn filters_order_deinterlace_resize_range() {
        let mut pr = base();
        pr.deinterlace = true;
        pr.resize = Some((1280, 720));
        pr.range_convert = true;
        let b = build(&pr).unwrap();
        let i = b.args.iter().position(|x| x == "-vf").unwrap();
        assert_eq!(b.args[i + 1], format!("{DEINTERLACE_FILTER},scale=1280:720,{RANGE_FILTER}"));
    }

    #[test]
    fn deinterlace_ignored_with_video_copy() {
        let mut pr = base();
        pr.deinterlace = true;
        pr.passthrough = Passthrough::Video;
        let b = build(&pr).unwrap();
        assert!(!b.args.iter().any(|x| x.contains("bwdif")));
        assert!(!b.warnings.is_empty());
    }

    #[test]
    fn chapters_are_second_input_and_mapped() {
        let mut pr = base();
        pr.trim_start = 5.0;
        pr.chapters = Some(("/tmp/ch.txt".into(), ";FFMETADATA1\n".into()));
        let b = build(&pr).unwrap();
        assert_eq!(&b.args[0..9], &["-y", "-ss", "5", "-i", "/in/My Video.mkv", "-f", "ffmetadata", "-i", "/tmp/ch.txt"]);
        assert!(b.args.windows(2).any(|w| w == ["-map_chapters", "1"]));
        assert_eq!(b.side_files.len(), 1);
    }

    #[test]
    fn mp4_output_handles_subs_attachments_faststart() {
        let mut pr = base();
        pr.output = "/out/x.mp4".into();
        pr.faststart = true;
        pr.has_attachments = true;
        pr.subtitle_streams = vec![
            StreamRef { index: 2, codec: "subrip".into() },
            StreamRef { index: 3, codec: "hdmv_pgs_subtitle".into() },
        ];
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-map 0 -map -0:3 -map -0:t"), "{s}");
        assert!(s.contains("-c:s mov_text -c:d copy -movflags +faststart /out/x.mp4"), "{s}");
        assert!(!s.contains("-c:t"));
        assert_eq!(b.warnings.len(), 2);
        // faststart ignored for mkv
        let mut pr = base();
        pr.faststart = true;
        assert!(!build(&pr).unwrap().args.contains(&"+faststart".to_string()));
    }

    #[test]
    fn default_ext() {
        assert_eq!(default_output_ext(std::path::Path::new("a.MKV")), "mkv");
        assert_eq!(default_output_ext(std::path::Path::new("a.avi")), "mp4");
    }
}
