//! Mode 2: MP4 from a still image + audio.

use std::path::PathBuf;

use super::common::*;
use super::codecs::{self, VideoCodec};
use super::{p, ArgVec, BuiltCommand};
use crate::util::fmt_num;

/// RGB/JPEG -> limited-range BT.709 YUV in a single scale step so the BT.709 tags are accurate.
pub const STILL_RANGE_FILTER: &str = "scale=out_range=tv:out_color_matrix=bt709,format=yuv420p";

#[derive(Debug, Clone, PartialEq)]
pub struct StillParams {
    pub image: PathBuf,
    pub audio: PathBuf,
    pub output: PathBuf,
    pub video: VideoEncode,
    pub audio_encode: AudioEncode,
    /// None = `-shortest` (use the audio length), Some(s) = `-t s`.
    pub duration_s: Option<f64>,
    pub faststart: bool,
    pub range_convert: bool,
}

/// Factory H.264 values for this mode: CRF 23 / preset medium / `-tune stillimage`.
/// `-crf` and `-preset` are omitted when those values are left at libx264's own defaults
/// so the default command stays `-c:v libx264 -tune stillimage -pix_fmt yuv420p`.
pub fn still_default_video() -> VideoEncode {
    VideoEncode { crf: 23, preset: "medium".into(), tune: Some("stillimage".into()), ..Default::default() }
}

/// H.264 still args: same as [`x264_args`] except `-crf 23` and `-preset medium` are omitted
/// (those are libx264's implicit defaults). Other codecs use [`codecs::video_args`].
pub fn still_video_args(v: &VideoEncode, filter: Option<&str>, range_tags: bool, output: &std::path::Path) -> Vec<String> {
    if v.codec != VideoCodec::H264 {
        return codecs::video_args(v, filter, range_tags, output);
    }
    let mut a: Vec<String> = Vec::new();
    a.push_all(["-c:v", "libx264"]);
    if v.crf != 23 {
        a.push("-crf".into());
        a.push(v.crf.to_string());
    }
    if v.preset != "medium" {
        a.push("-preset".into());
        a.push(v.preset.clone());
    }
    if let Some(t) = v.tune.as_ref().filter(|t| !t.is_empty()) {
        a.push_all(["-tune", t.as_str()]);
    }
    a.push_all(["-pix_fmt", "yuv420p"]);
    if let Some(f) = with_output_fps(filter, v) {
        a.push("-vf".into());
        a.push(f);
    }
    if range_tags {
        a.extend(color_tag_args());
    }
    a
}

fn still_range_filter(v: &VideoEncode) -> String {
    if v.codec == VideoCodec::H264 || !v.ten_bit {
        STILL_RANGE_FILTER.to_string()
    } else {
        "scale=out_range=tv:out_color_matrix=bt709,format=yuv420p10le".into()
    }
}

pub fn build(pr: &StillParams) -> Result<BuiltCommand, String> {
    if let Some(d) = pr.duration_s {
        if d <= 0.0 {
            return Err(format!("Invalid duration: {d}"));
        }
    }
    let mut warnings = codecs::check_video(&pr.video, &pr.output)?;
    warnings.extend(codecs::check_audio(&pr.audio_encode, &pr.output)?);
    let mut a: Vec<String> = Vec::new();
    a.push_all(["-y", "-loop", "1", "-i"]);
    a.push(p(&pr.image));
    a.push("-i".into());
    a.push(p(&pr.audio));
    let filter = pr.range_convert.then(|| still_range_filter(&pr.video));
    a.extend(still_video_args(&pr.video, filter.as_deref(), pr.range_convert, &pr.output));
    a.extend(codecs::audio_args(&pr.audio_encode));
    if let Some(f) = pr.audio_encode.filter_chain() {
        a.push_all(["-af", &f]);
    }
    match pr.duration_s {
        None => a.push("-shortest".into()),
        Some(d) => a.push_all(["-t".to_string(), fmt_num(d)]),
    }
    // like subs / tracks / join / frameserve / custom: -movflags only means something for MP4 / MOV
    if pr.faststart && super::reencode::is_mp4_like(&pr.output) {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, warnings, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> StillParams {
        StillParams {
            image: "/in/cover art.png".into(),
            audio: "/in/song.flac".into(),
            output: "/out/cover art_still.mp4".into(),
            video: still_default_video(),
            audio_encode: AudioEncode { bitrate_k: 192, ..Default::default() },
            duration_s: None,
            faststart: false,
            range_convert: false,
        }
    }
    #[test]
    fn shortest_default() {
        let b = build(&base()).unwrap();
        assert_eq!(
            b.args,
            vec!["-y", "-loop", "1", "-i", "/in/cover art.png", "-i", "/in/song.flac", "-c:v", "libx264", "-tune",
                 "stillimage", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest", "/out/cover art_still.mp4"]
        );
    }
    #[test]
    fn faststart_only_for_mp4_like_output() {
        let mut pr = base();
        pr.faststart = true;
        pr.output = "/out/cover art_still.mkv".into();
        let a = build(&pr).unwrap().args;
        assert!(!a.iter().any(|x| x == "-movflags"), "{a:?}");
        assert_eq!(a.last().unwrap(), "/out/cover art_still.mkv");
        for ext in ["mp4", "mov", "m4v"] {
            pr.output = format!("/out/x.{ext}").into();
            let s = build(&pr).unwrap().args.join(" ");
            assert!(s.contains(&format!("-movflags +faststart /out/x.{ext}")), "{s}");
        }
        // faststart off: identical commands for MP4 and MKV apart from the file name
        pr.faststart = false;
        pr.output = "/out/x.mkv".into();
        let mkv = build(&pr).unwrap().args;
        pr.output = "/out/x.mp4".into();
        let mp4 = build(&pr).unwrap().args;
        assert_eq!(mkv[..mkv.len() - 1], mp4[..mp4.len() - 1]);
    }
    #[test]
    fn custom_duration_faststart_range() {
        let mut pr = base();
        pr.duration_s = Some(300.5);
        pr.faststart = true;
        pr.range_convert = true;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-t 300.5 -movflags +faststart /out/cover art_still.mp4"));
        assert!(s.contains(STILL_RANGE_FILTER));
        assert!(s.contains("-color_range tv -colorspace bt709"));
        pr.duration_s = Some(0.0);
        assert!(build(&pr).is_err());
    }

    #[test]
    fn default_command_omits_implicit_crf_and_preset() {
        let s = build(&base()).unwrap().args.join(" ");
        assert!(!s.contains("-crf"), "{s}");
        assert!(!s.contains("-preset"), "{s}");
        assert!(s.contains("-c:v libx264 -tune stillimage -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("-c:a aac -b:a 192k"), "{s}");
    }

    #[test]
    fn custom_crf_is_emitted_and_hevc_uses_video_args() {
        let mut pr = base();
        pr.video.crf = 20;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-c:v libx264 -crf 20 -tune stillimage") || s.contains("-crf 20"), "{s}");
        pr.video = VideoEncode { codec: VideoCodec::Hevc, crf: 18, preset: "slow".into(), ten_bit: true, tune: None, ..Default::default() };
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-c:v libx265"), "{s}");
        assert!(s.contains("-crf 18"), "{s}");
    }
}
