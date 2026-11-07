//! Extra video/audio codecs (codecs branch): HEVC (libx265) and AV1 (libsvtav1) next to the default
//! H.264 (libx264), and AC3 / copy next to the default AAC. H.264 + AAC go through the unchanged
//! 0.5.x builders (`x264_args`, `aac_args`), so existing presets build byte-for-byte the same commands.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::common::{color_tag_args, x264_args, AudioEncode, VideoEncode};
use super::ArgVec;
use crate::util::ext_lower;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VideoCodec {
    /// libx264 – plays everywhere (the default).
    #[default]
    H264,
    /// libx265
    Hevc,
    /// libsvtav1
    Av1,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1];
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264 (libx264) – default, plays everywhere",
            VideoCodec::Hevc => "HEVC / H.265 (libx265)",
            VideoCodec::Av1 => "AV1 (SVT-AV1)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Av1 => "AV1",
        }
    }
    pub fn encoder(self) -> &'static str {
        match self {
            VideoCodec::H264 => "libx264",
            VideoCodec::Hevc => "libx265",
            VideoCodec::Av1 => "libsvtav1",
        }
    }
    /// CRF range accepted by the encoder.
    pub fn crf_range(self) -> std::ops::RangeInclusive<u32> {
        match self {
            VideoCodec::Av1 => 0..=63,
            _ => 0..=51,
        }
    }
    /// Suggested CRF when switching to this codec (game footage / camera footage).
    pub fn default_crf(self, camera: bool) -> u32 {
        match (self, camera) {
            (VideoCodec::H264, _) => 22,
            (VideoCodec::Hevc, false) => 20,
            (VideoCodec::Hevc, true) => 18,
            (VideoCodec::Av1, false) => 28,
            (VideoCodec::Av1, true) => 26,
        }
    }
    /// Plain-language playback note.
    pub fn compat_note(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264 plays everywhere: TVs, phones, tablets, every browser and Plex without converting.",
            VideoCodec::Hevc => "HEVC: great on most smart TVs, Apple devices and newer phones (Plex usually plays it directly). Chrome and Firefox usually can't play it, so Plex may convert (transcode) it for browsers.",
            VideoCodec::Av1 => "AV1: Chrome and Firefox play it, and newer phones/TVs. Many older TVs, streaming sticks and phones can't, so Plex may convert (transcode) it for them.",
        }
    }
}

pub const CRF_NOTE: &str = "CRF numbers aren't comparable across codecs: HEVC CRF 20 or AV1 CRF 28 look roughly like H.264 CRF 18–22 at a much smaller size. Lower = better quality / bigger file.";

/// x265 presets.
pub const X265_PRESETS: [&str; 10] = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow", "placebo"];
/// x265 tunes offered (x264's film/stillimage don't exist in x265).
pub const X265_TUNES: [&str; 4] = ["grain", "animation", "fastdecode", "zerolatency"];
pub const SVT_PRESET_MAX: u32 = 13;
pub const FILM_GRAIN_MAX: u32 = 50;

/// Pixel format written for HEVC/AV1.
pub fn pix_fmt(ten_bit: bool) -> &'static str {
    if ten_bit {
        "yuv420p10le"
    } else {
        "yuv420p"
    }
}

fn mp4_like_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "m4v" | "mov")
}

/// Video encoder arguments for any codec. H.264 = exactly [`x264_args`].
pub fn video_args(v: &VideoEncode, filter: Option<&str>, range_tags: bool, output: &Path) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let vf = |a: &mut Vec<String>| {
        if let Some(f) = super::common::with_output_fps(filter, v) {
            a.push("-vf".into());
            a.push(f);
        }
        if range_tags {
            a.extend(color_tag_args());
        }
    };
    match v.codec {
        VideoCodec::H264 => return x264_args(v, filter, range_tags),
        VideoCodec::Hevc => {
            a.push_all(["-c:v", "libx265", "-crf"]);
            a.push(v.crf.to_string());
            a.push("-preset".into());
            a.push(v.preset.clone());
            a.push_all(["-pix_fmt", pix_fmt(v.ten_bit)]);
            vf(&mut a);
            if let Some(t) = v.tune.as_ref().filter(|t| !t.is_empty()) {
                a.push_all(["-tune", t.as_str()]);
            }
            // keep x265's per-frame chatter out of the log
            a.push_all(["-x265-params", "log-level=error"]);
            if mp4_like_ext(&ext_lower(output)) {
                // Apple devices / many TVs only play HEVC in MP4/MOV with the hvc1 tag
                a.push_all(["-tag:v", "hvc1"]);
            }
        }
        VideoCodec::Av1 => {
            a.push_all(["-c:v", "libsvtav1", "-crf"]);
            a.push(v.crf.to_string());
            a.push("-preset".into());
            a.push(v.svt_preset.to_string());
            a.push_all(["-pix_fmt", pix_fmt(v.ten_bit)]);
            vf(&mut a);
            if let Some(p) = svtav1_params(v) {
                a.push_all(["-svtav1-params".to_string(), p]);
            }
        }
    }
    a
}

/// `film-grain=N:film-grain-denoise=0|1`, or None when film grain synthesis is off.
pub fn svtav1_params(v: &VideoEncode) -> Option<String> {
    (v.film_grain > 0).then(|| format!("film-grain={}:film-grain-denoise={}", v.film_grain, v.film_grain_denoise as u8))
}

/// Check codec settings and the output container. Ok(warnings) / Err(plain message).
pub fn check_video(v: &VideoEncode, output: &Path) -> Result<Vec<String>, String> {
    let mut w = vec![];
    let c = v.codec;
    if c == VideoCodec::H264 {
        return Ok(w); // unchanged 0.5.x behaviour
    }
    if !c.crf_range().contains(&v.crf) {
        return Err(format!("{} CRF must be {}–{} (it is {}).", c.short(), c.crf_range().start(), c.crf_range().end(), v.crf));
    }
    match c {
        VideoCodec::Hevc => {
            if !X265_PRESETS.contains(&v.preset.as_str()) {
                return Err(format!("“{}” isn't an x265 preset (use ultrafast … placebo).", v.preset));
            }
            if let Some(t) = v.tune.as_ref().filter(|t| !t.is_empty()) {
                if !X265_TUNES.contains(&t.as_str()) {
                    return Err(format!("Tune “{t}” doesn't exist for HEVC (x265). Choose none, grain, animation, fastdecode or zerolatency."));
                }
            }
        }
        VideoCodec::Av1 => {
            if v.svt_preset > SVT_PRESET_MAX {
                return Err(format!("AV1 preset must be 0–{SVT_PRESET_MAX} (it is {}).", v.svt_preset));
            }
            if v.film_grain > FILM_GRAIN_MAX {
                return Err(format!("AV1 film grain must be 0–{FILM_GRAIN_MAX} (it is {}).", v.film_grain));
            }
        }
        VideoCodec::H264 => {}
    }
    let ext = ext_lower(output);
    match (c, ext.as_str()) {
        (_, "mp4" | "m4v" | "mkv") => {}
        (VideoCodec::Hevc, "mov") => {}
        (VideoCodec::Av1, "mov") => w.push("AV1 in .mov plays in very few apps (QuickTime can't). MP4 or MKV is safer.".into()),
        (_, e) => return Err(format!("{} can't be saved as .{e} here. Use MP4 or MKV.", c.short())),
    }
    w.push(match c {
        VideoCodec::Hevc => "HEVC: Chrome/Firefox usually can't play this file; Plex may convert it for browsers.".into(),
        _ => "AV1: many older TVs and phones can't play this file; Plex may convert it for them.".into(),
    });
    Ok(w)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioCodec {
    /// AAC (default)
    #[default]
    Aac,
    /// Dolby Digital (AC3), up to 5.1
    Ac3,
    /// Keep the original audio (stream copy)
    Copy,
}

impl AudioCodec {
    pub const ALL: [AudioCodec; 3] = [AudioCodec::Aac, AudioCodec::Ac3, AudioCodec::Copy];
    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Aac => "AAC (default)",
            AudioCodec::Ac3 => "AC3 / Dolby Digital (up to 5.1)",
            AudioCodec::Copy => "Keep original audio (copy)",
        }
    }
}

pub const AC3_BITRATES: [u32; 3] = [384, 448, 640];

/// Audio encoder arguments. AAC = exactly [`AudioEncode::aac_args`].
pub fn audio_args(au: &AudioEncode) -> Vec<String> {
    match au.codec {
        AudioCodec::Aac => au.aac_args(),
        AudioCodec::Ac3 => {
            let mut a: Vec<String> = vec!["-c:a".into(), "ac3".into(), "-b:a".into(), format!("{}k", au.ac3_bitrate_k)];
            // AC3 holds at most 5.1: downmix 7.1 etc. (5.1 and less pass unchanged)
            if au.source_channels.is_some_and(|c| c > 6) {
                a.push_all(["-ac", "6"]);
            }
            a
        }
        AudioCodec::Copy => vec!["-c:a".into(), "copy".into()],
    }
}

/// Check the audio choice. Ok(warnings) / Err(plain message).
pub fn check_audio(au: &AudioEncode, output: &Path) -> Result<Vec<String>, String> {
    let mut w = vec![];
    match au.codec {
        AudioCodec::Aac => {}
        AudioCodec::Ac3 => {
            if !(96..=640).contains(&au.ac3_bitrate_k) {
                return Err(format!("AC3 bitrate must be 96–640 kbps (it is {}).", au.ac3_bitrate_k));
            }
            if au.source_channels.is_some_and(|c| c > 6) {
                w.push("AC3 holds up to 5.1: the audio is downmixed to 5.1.".into());
            }
        }
        AudioCodec::Copy => {
            if au.filter_chain().is_some() {
                w.push("Keep original audio: gain and compressor can't be applied to copied audio and are ignored.".into());
            }
            if mp4_like_ext(&ext_lower(output)) {
                w.push("Keep original audio into MP4: works for AAC/AC3/MP3 sources; other audio (e.g. PCM, FLAC in older players, Opus) may fail or not play – MKV keeps anything.".into());
            }
        }
    }
    Ok(w)
}

/// Which optional encoders an FFmpeg build has (from `ffmpeg -encoders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Encoders {
    pub libx265: bool,
    pub libsvtav1: bool,
    pub ac3: bool,
}

impl Encoders {
    pub fn from_list(encoders_output: &str) -> Self {
        let has = |n: &str| crate::ffmpeg::version::has_encoder(encoders_output, n);
        Encoders { libx265: has("libx265"), libsvtav1: has("libsvtav1"), ac3: has("ac3") }
    }
    pub fn video(&self, c: VideoCodec) -> bool {
        match c {
            VideoCodec::H264 => true,
            VideoCodec::Hevc => self.libx265,
            VideoCodec::Av1 => self.libsvtav1,
        }
    }
    pub fn audio(&self, c: AudioCodec) -> bool {
        c != AudioCodec::Ac3 || self.ac3
    }
}

/// Plain explanation for a missing encoder.
pub fn missing_note(encoder: &str) -> String {
    format!("This FFmpeg has no {encoder} encoder. Help › Check for FFmpeg updates… can download a full build (BtbN GPL builds include it).")
}

/// Probe-based note: 10-bit output from an 8-bit source.
pub fn ten_bit_note(source_pix_fmt: Option<&str>, v: &VideoEncode) -> Option<String> {
    if v.codec == VideoCodec::H264 || !v.ten_bit {
        return None;
    }
    let pf = source_pix_fmt?;
    let eight = !(pf.contains("10") || pf.contains("12") || pf.contains("16"));
    Some(if eight {
        format!("The source is 8-bit ({pf}). 10-bit output is still fine: it adds no detail, but it protects gradients (sky, dark scenes) from banding at a similar size.")
    } else {
        format!("The source is high bit depth ({pf}); 10-bit keeps it.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn v(codec: VideoCodec) -> VideoEncode {
        VideoEncode { codec, crf: codec.default_crf(false), ..Default::default() }
    }
    fn s(a: &[String]) -> String {
        a.join(" ")
    }

    #[test]
    fn defaults_are_h264_and_aac() {
        assert_eq!(VideoEncode::default().codec, VideoCodec::H264);
        assert_eq!(AudioEncode::default().codec, AudioCodec::Aac);
        // old presets (no codec keys) load as H.264 / AAC
        let old: VideoEncode = serde_json::from_str(r#"{"crf": 20, "preset": "slow", "tune": null}"#).unwrap();
        assert_eq!(old.codec, VideoCodec::H264);
        let olda: AudioEncode = serde_json::from_str(r#"{"bitrate_k": 256}"#).unwrap();
        assert_eq!(olda.codec, AudioCodec::Aac);
        let p = PathBuf::from("/o/a.mp4");
        assert_eq!(video_args(&VideoEncode::default(), Some("bwdif"), true, &p), x264_args(&VideoEncode::default(), Some("bwdif"), true));
        assert_eq!(audio_args(&AudioEncode::default()), AudioEncode::default().aac_args());
    }

    #[test]
    fn hevc_args_per_container() {
        let mut h = v(VideoCodec::Hevc);
        assert_eq!(s(&video_args(&h, None, false, Path::new("/o/a.mp4"))), "-c:v libx265 -crf 20 -preset slow -pix_fmt yuv420p10le -x265-params log-level=error -tag:v hvc1");
        assert_eq!(s(&video_args(&h, None, false, Path::new("/o/a.MOV"))), "-c:v libx265 -crf 20 -preset slow -pix_fmt yuv420p10le -x265-params log-level=error -tag:v hvc1");
        h.ten_bit = false;
        h.tune = Some("grain".into());
        assert_eq!(s(&video_args(&h, Some("scale=1280:720"), true, Path::new("/o/a.mkv"))), "-c:v libx265 -crf 20 -preset slow -pix_fmt yuv420p -vf scale=1280:720 -color_range tv -colorspace bt709 -color_primaries bt709 -color_trc bt709 -tune grain -x265-params log-level=error");
    }

    #[test]
    fn av1_args_and_film_grain() {
        let mut a = v(VideoCodec::Av1);
        assert_eq!(s(&video_args(&a, None, false, Path::new("/o/a.mp4"))), "-c:v libsvtav1 -crf 28 -preset 5 -pix_fmt yuv420p10le");
        a.film_grain = 8;
        assert_eq!(s(&video_args(&a, None, false, Path::new("/o/a.mkv"))), "-c:v libsvtav1 -crf 28 -preset 5 -pix_fmt yuv420p10le -svtav1-params film-grain=8:film-grain-denoise=0");
        a.film_grain_denoise = true;
        a.tune = Some("grain".into()); // no tune for AV1: never emitted
        assert_eq!(svtav1_params(&a).unwrap(), "film-grain=8:film-grain-denoise=1");
        assert!(!video_args(&a, None, false, Path::new("/o/a.mkv")).contains(&"-tune".to_string()));
    }

    #[test]
    fn validation() {
        let p = Path::new("/o/a.mp4");
        assert!(check_video(&VideoEncode { crf: 99, ..Default::default() }, p).unwrap().is_empty(), "H.264 unchanged: no new checks");
        let mut h = v(VideoCodec::Hevc);
        assert!(check_video(&h, p).unwrap().iter().any(|w| w.contains("Chrome")));
        h.preset = "placebo".into();
        assert!(check_video(&h, p).is_ok());
        h.tune = Some("film".into());
        assert!(check_video(&h, p).unwrap_err().contains("doesn't exist for HEVC"));
        h.tune = None;
        h.crf = 52;
        assert!(check_video(&h, p).is_err());
        h.crf = 20;
        h.preset = "7".into();
        assert!(check_video(&h, p).is_err());
        let mut a = v(VideoCodec::Av1);
        a.crf = 63;
        assert!(check_video(&a, p).is_ok());
        a.crf = 64;
        assert!(check_video(&a, p).is_err());
        a.crf = 30;
        a.svt_preset = 14;
        assert!(check_video(&a, p).is_err());
        a.svt_preset = 5;
        a.film_grain = 51;
        assert!(check_video(&a, p).is_err());
        a.film_grain = 0;
        assert!(check_video(&a, Path::new("/o/a.mov")).unwrap().iter().any(|w| w.contains(".mov")));
        assert!(check_video(&a, Path::new("/o/a.avi")).unwrap_err().contains("MP4 or MKV"));
        assert!(check_video(&v(VideoCodec::Hevc), Path::new("/o/a.webm")).is_err());
    }

    #[test]
    fn audio_choices() {
        let mut au = AudioEncode { codec: AudioCodec::Ac3, ..Default::default() };
        assert_eq!(s(&audio_args(&au)), "-c:a ac3 -b:a 448k");
        au.source_channels = Some(8);
        assert_eq!(s(&audio_args(&au)), "-c:a ac3 -b:a 448k -ac 6");
        assert!(check_audio(&au, Path::new("/o/a.mkv")).unwrap()[0].contains("downmixed"));
        au.source_channels = Some(6);
        assert_eq!(s(&audio_args(&au)), "-c:a ac3 -b:a 448k", "5.1 kept");
        au.ac3_bitrate_k = 1000;
        assert!(check_audio(&au, Path::new("/o/a.mkv")).is_err());
        let mut c = AudioEncode { codec: AudioCodec::Copy, gain_db: 3.0, ..Default::default() };
        assert_eq!(s(&audio_args(&c)), "-c:a copy");
        let w = check_audio(&c, Path::new("/o/a.mp4")).unwrap();
        assert!(w.iter().any(|x| x.contains("ignored")) && w.iter().any(|x| x.contains("MKV keeps")));
        c.gain_db = 0.0;
        assert!(check_audio(&c, Path::new("/o/a.mkv")).unwrap().is_empty());
    }

    #[test]
    fn encoder_detection_parsing() {
        let out = " V....D libx264              libx264 H.264\n V....D libx265              libx265 H.265 / HEVC (codec hevc)\n A....D ac3                  ATSC A/52A (AC-3)\n A....D ac3_fixed            ATSC A/52A (AC-3) (codec ac3)\n";
        let e = Encoders::from_list(out);
        assert_eq!(e, Encoders { libx265: true, libsvtav1: false, ac3: true });
        assert!(e.video(VideoCodec::H264) && e.video(VideoCodec::Hevc) && !e.video(VideoCodec::Av1));
        let none = Encoders::from_list(" A....D ac3_fixed ATSC (codec ac3)\n");
        assert!(!none.ac3, "ac3_fixed is not ac3");
    }

    #[test]
    fn ten_bit_notes() {
        let h = v(VideoCodec::Hevc);
        assert!(ten_bit_note(Some("yuv420p"), &h).unwrap().contains("8-bit"));
        assert!(ten_bit_note(Some("yuv420p10le"), &h).unwrap().contains("keeps"));
        assert_eq!(ten_bit_note(Some("yuv420p"), &VideoEncode::default()), None);
        assert_eq!(ten_bit_note(None, &h), None);
    }
}
