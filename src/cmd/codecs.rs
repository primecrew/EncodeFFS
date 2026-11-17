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
/// Useful SVT-AV1 speeds shown in the Speed drop-down (Custom… covers the rest of 0–13).
pub const SVT_SPEEDS: [u32; 8] = [2, 4, 5, 6, 8, 10, 12, 13];
pub const FILM_GRAIN_MAX: u32 = 50;

/// Closed drop-down / menu label for an SVT-AV1 speed. Unlisted values show as "Custom N".
pub fn svt_speed_label(n: u32) -> String {
    match n {
        2 => "2 – very slow, smallest file".into(),
        4 => "4 – slow".into(),
        5 => "5 – archive default".into(),
        6 => "6".into(),
        8 => "8 – fast".into(),
        10 => "10 – very fast, bigger file".into(),
        12 => "12 – fastest, for tests".into(),
        13 => "13 – fastest, for tests".into(),
        n => format!("Custom {n}"),
    }
}

/// Pixel format written for HEVC/AV1.
pub fn pix_fmt(ten_bit: bool) -> &'static str {
    if ten_bit {
        "yuv420p10le"
    } else {
        "yuv420p"
    }
}

/// HEVC/AV1 with 10-bit output (H.264 always writes 8-bit yuv420p).
pub fn ten_bit_output(v: &VideoEncode) -> bool {
    v.codec != VideoCodec::H264 && v.ten_bit
}

/// The full-range → TV-range filter. For 10-bit HEVC/AV1 the squeeze and the 10-bit
/// conversion happen in one step (`scale=in_range=full:out_range=tv,format=yuv420p10le`
/// or `format=<pix>` when a non-4:2:0 layout is requested) so the squeeze is not done
/// at 8-bit. H.264 and 8-bit paths return the historic [`RANGE_FILTER`] string unchanged.
pub fn range_filter(v: &VideoEncode) -> String {
    range_filter_pix(v, pix_fmt(true))
}

/// Like [`range_filter`], but uses `pix` (e.g. `yuv422p10le`) when squeezing at 10-bit.
pub fn range_filter_pix(v: &VideoEncode, pix: &str) -> String {
    if ten_bit_output(v) {
        format!("{},format={}", super::common::RANGE_FILTER, pix)
    } else {
        super::common::RANGE_FILTER.to_string()
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

/// Typical AAC bitrates in the dropdown. Any other value is shown as Custom.
pub const AAC_BITRATES: [u32; 6] = [96, 128, 160, 192, 256, 320];
/// Typical AC3 bitrates in the dropdown (stereo through 5.1). Any other value in 96–640 is Custom.
pub const AC3_BITRATES: [u32; 5] = [192, 256, 384, 448, 640];

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

/// Plain explanation for a missing encoder, naming which FFmpeg is in use when known.
pub fn missing_note_for(encoder: &str, ff: Option<&crate::ffmpeg::locate::FfPaths>) -> String {
    let which = match ff {
        Some(p) if p.is_app_managed() => format!("The app-managed FFmpeg ({})", p.bin_dir.display()),
        Some(p) => format!("Your FFmpeg ({})", p.bin_dir.display()),
        None => "This FFmpeg".into(),
    };
    format!("{which} has no {encoder} encoder. Help › Check for FFmpeg updates… can download a full build (BtbN GPL builds include it).")
}

/// Plain explanation for a missing encoder when the in-use FFmpeg is not known.
pub fn missing_note(encoder: &str) -> String {
    missing_note_for(encoder, None)
}

/// True when the FFmpeg in use lacks `encoder` but the app-managed copy has it (and is not already in use).
pub fn can_switch_to_managed(
    current: Option<&crate::ffmpeg::locate::FfPaths>,
    current_enc: Option<&Encoders>,
    managed_enc: Option<&Encoders>,
    encoder: &str,
) -> bool {
    let Some(enc) = current_enc else { return false };
    let Some(managed) = managed_enc else { return false };
    if current.is_some_and(|p| p.is_app_managed()) {
        return false;
    }
    let missing_now = match encoder {
        "libx265" => !enc.libx265,
        "libsvtav1" => !enc.libsvtav1,
        "ac3" => !enc.ac3,
        _ => return false,
    };
    let has_managed = match encoder {
        "libx265" => managed.libx265,
        "libsvtav1" => managed.libsvtav1,
        "ac3" => managed.ac3,
        _ => false,
    };
    missing_now && has_managed
}

/// True when any of HEVC / AV1 / AC3 is missing from the in-use build but present in the app-managed copy.
pub fn can_switch_to_managed_for_any(
    current: Option<&crate::ffmpeg::locate::FfPaths>,
    current_enc: Option<&Encoders>,
    managed_enc: Option<&Encoders>,
) -> bool {
    switch_under_video(current, current_enc, managed_enc) || switch_under_audio(current, current_enc, managed_enc)
}

/// Show the switch button under the video picker when a missing video encoder is in the managed copy.
pub fn switch_under_video(
    current: Option<&crate::ffmpeg::locate::FfPaths>,
    current_enc: Option<&Encoders>,
    managed_enc: Option<&Encoders>,
) -> bool {
    ["libx265", "libsvtav1"].iter().any(|e| can_switch_to_managed(current, current_enc, managed_enc, e))
}

/// Show the switch button under the audio picker only when AC3 is the reason (not also a video encoder).
pub fn switch_under_audio(
    current: Option<&crate::ffmpeg::locate::FfPaths>,
    current_enc: Option<&Encoders>,
    managed_enc: Option<&Encoders>,
) -> bool {
    !switch_under_video(current, current_enc, managed_enc)
        && can_switch_to_managed(current, current_enc, managed_enc, "ac3")
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


/// Which FFmpeg a job should run with when the chosen encoder may be missing
/// from the configured copy. Never modifies the user's own FFmpeg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncoderRuntime {
    /// The FFmpeg in use has the encoder (or none is needed).
    Current,
    /// Use the app-managed copy for this encode only; leave the user's folder as it is.
    UseManaged { short: &'static str },
    /// The app-managed copy is not installed yet. Offer to download it.
    NeedDownload { short: &'static str },
    /// Neither copy has the encoder.
    Unavailable { short: &'static str },
}

impl EncoderRuntime {
    pub fn short(&self) -> Option<&'static str> {
        match self {
            EncoderRuntime::Current => None,
            EncoderRuntime::UseManaged { short } | EncoderRuntime::NeedDownload { short } | EncoderRuntime::Unavailable { short } => Some(*short),
        }
    }
}

pub fn encoder_short(encoder: &str) -> &'static str {
    match encoder {
        "libx265" => "HEVC",
        "libsvtav1" => "AV1",
        "ac3" => "AC3",
        "libx264" => "H.264",
        _ => "this codec",
    }
}

/// Exact prompt when the managed copy still needs to be downloaded.
pub fn download_prompt(short: &str) -> String {
    format!("Your FFmpeg doesn't support {short}. Download the current FFmpeg build?")
}

/// Note on the form when this encode will use the app-managed copy.
pub fn use_managed_note(short: &str) -> String {
    format!("Your FFmpeg doesn't have {short}. This encode will use the app-managed FFmpeg. Your FFmpeg is left as it is.")
}

/// Note on the form when a download will be offered at Run.
pub fn need_download_note(short: &str) -> String {
    format!("Your FFmpeg doesn't have {short}. When you run the encode, you can download the current FFmpeg build (into the app-managed folder). Your FFmpeg is left as it is.")
}

fn enc_has(enc: &Encoders, encoder: &str) -> bool {
    match encoder {
        "libx264" => true,
        "libx265" => enc.libx265,
        "libsvtav1" => enc.libsvtav1,
        "ac3" => enc.ac3,
        _ => true,
    }
}

/// Encoders a `VideoEncode` / `AudioEncode` pair will actually invoke.
pub fn needed_encoders(v: Option<&VideoEncode>, a: Option<&AudioEncode>) -> Vec<&'static str> {
    let mut n = Vec::new();
    if let Some(v) = v {
        match v.codec {
            VideoCodec::Hevc => n.push("libx265"),
            VideoCodec::Av1 => n.push("libsvtav1"),
            VideoCodec::H264 => {}
        }
    }
    if let Some(a) = a {
        if a.codec == AudioCodec::Ac3 {
            n.push("ac3");
        }
    }
    n
}

/// Encoders mentioned in already-built ffmpeg arguments (`-c:v libsvtav1`, `-c:a ac3`, …).
pub fn encoders_in_args(args: &[String]) -> Vec<&'static str> {
    let mut n = Vec::new();
    for pair in args.windows(2) {
        if pair[0] == "-c:v" || pair[0] == "-codec:v" || pair[0] == "-vcodec" {
            match pair[1].as_str() {
                "libx265" if !n.contains(&"libx265") => n.push("libx265"),
                "libsvtav1" if !n.contains(&"libsvtav1") => n.push("libsvtav1"),
                _ => {}
            }
        }
        if pair[0] == "-c:a" || pair[0] == "-codec:a" || pair[0] == "-acodec" {
            if pair[1] == "ac3" && !n.contains(&"ac3") {
                n.push("ac3");
            }
        }
    }
    n
}

fn join_shorts(encoders: &[&str]) -> &'static str {
    // Only the three optional encoders are ever missing. Keep a static string
    // so the prompt does not allocate a new label type.
    match encoders {
        ["libx265"] => "HEVC",
        ["libsvtav1"] => "AV1",
        ["ac3"] => "AC3",
        ["libx265", "ac3"] => "HEVC or AC3",
        ["libsvtav1", "ac3"] => "AV1 or AC3",
        ["libx265", "libsvtav1"] => "HEVC or AV1",
        ["libx265", "libsvtav1", "ac3"] => "HEVC, AV1 or AC3",
        [] => "",
        _ => encoder_short(encoders[0]),
    }
}

/// Decide which FFmpeg should run the job. `managed_installed` is true when the
/// app-managed folder already contains ffmpeg and ffprobe.
pub fn encoder_runtime(
    needed: &[&str],
    current: Option<&crate::ffmpeg::locate::FfPaths>,
    current_enc: Option<&Encoders>,
    managed_enc: Option<&Encoders>,
    managed_installed: bool,
) -> EncoderRuntime {
    let missing: Vec<&str> = needed
        .iter()
        .copied()
        .filter(|e| current_enc.is_none_or(|enc| !enc_has(enc, e)))
        .collect();
    if missing.is_empty() {
        return EncoderRuntime::Current;
    }
    let short = join_shorts(&missing);
    if current.is_some_and(|p| p.is_app_managed()) {
        return EncoderRuntime::Unavailable { short };
    }
    if managed_enc.is_some_and(|m| missing.iter().all(|e| enc_has(m, e))) {
        return EncoderRuntime::UseManaged { short };
    }
    if !managed_installed {
        return EncoderRuntime::NeedDownload { short };
    }
    // Managed is present but lacks the encoder (or has not been probed yet).
    if managed_enc.is_none() {
        return EncoderRuntime::UseManaged { short };
    }
    EncoderRuntime::Unavailable { short }
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
    fn av1_speed_labels() {
        assert_eq!(SVT_SPEEDS, [2, 4, 5, 6, 8, 10, 12, 13]);
        assert_eq!(svt_speed_label(2), "2 – very slow, smallest file");
        assert_eq!(svt_speed_label(5), "5 – archive default");
        assert_eq!(svt_speed_label(6), "6");
        assert_eq!(svt_speed_label(13), "13 – fastest, for tests");
        assert_eq!(svt_speed_label(0), "Custom 0");
        assert_eq!(svt_speed_label(3), "Custom 3");
        assert_eq!(svt_speed_label(11), "Custom 11");
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
    fn missing_note_names_the_ffmpeg_in_use() {
        assert!(missing_note("libx265").starts_with("This FFmpeg has no libx265 encoder."));
        let own = crate::ffmpeg::locate::FfPaths {
            ffmpeg: "x".into(),
            ffprobe: "y".into(),
            bin_dir: std::path::PathBuf::from(r"D:\Tools\FFmpeg\bin"),
            source: crate::ffmpeg::locate::FfSource::Configured,
        };
        let n = missing_note_for("libx265", Some(&own));
        assert!(n.starts_with(r"Your FFmpeg (D:\Tools\FFmpeg\bin) has no libx265 encoder."), "{n}");
        assert!(n.contains("Help › Check for FFmpeg updates…"));
        let mg = crate::ffmpeg::locate::FfPaths { source: crate::ffmpeg::locate::FfSource::Managed, ..own };
        let n = missing_note_for("libsvtav1", Some(&mg));
        assert!(n.starts_with("The app-managed FFmpeg ("), "{n}");
        assert!(n.contains("has no libsvtav1 encoder"));
    }

    #[test]
    fn switch_is_offered_only_when_managed_has_the_missing_encoder() {
        let own = crate::ffmpeg::locate::FfPaths {
            ffmpeg: "x".into(),
            ffprobe: "y".into(),
            bin_dir: std::path::PathBuf::from(r"D:\mine\bin"),
            source: crate::ffmpeg::locate::FfSource::Configured,
        };
        let thin = Encoders { libx265: false, libsvtav1: false, ac3: false };
        let full = Encoders { libx265: true, libsvtav1: true, ac3: true };
        assert!(can_switch_to_managed(Some(&own), Some(&thin), Some(&full), "libx265"));
        assert!(can_switch_to_managed_for_any(Some(&own), Some(&thin), Some(&full)));
        assert!(!can_switch_to_managed(Some(&own), Some(&thin), Some(&thin), "libx265"), "managed also lacks it");
        assert!(!can_switch_to_managed(Some(&own), Some(&full), Some(&full), "libx265"), "already has it");
        assert!(!can_switch_to_managed(None, Some(&thin), Some(&full), "libx264"), "unknown encoder / H.264");
        let mg = crate::ffmpeg::locate::FfPaths { source: crate::ffmpeg::locate::FfSource::Managed, ..own.clone() };
        assert!(!can_switch_to_managed(Some(&mg), Some(&thin), Some(&full), "libx265"), "already on managed");
        assert!(!can_switch_to_managed(Some(&own), None, Some(&full), "libx265"), "current encoders unknown");
        assert!(!can_switch_to_managed(Some(&own), Some(&thin), None, "libx265"), "managed not probed");

        // One button per form: prefer the video picker; only AC3 missing → audio picker.
        assert!(switch_under_video(Some(&own), Some(&thin), Some(&full)));
        assert!(!switch_under_audio(Some(&own), Some(&thin), Some(&full)), "video takes priority when both are missing");
        let no_video = Encoders { libx265: true, libsvtav1: true, ac3: false };
        assert!(!switch_under_video(Some(&own), Some(&no_video), Some(&full)));
        assert!(switch_under_audio(Some(&own), Some(&no_video), Some(&full)));
        let hevc_only = Encoders { libx265: false, libsvtav1: true, ac3: true };
        assert!(switch_under_video(Some(&own), Some(&hevc_only), Some(&full)));
        assert!(!switch_under_audio(Some(&own), Some(&hevc_only), Some(&full)));
    }

    #[test]
    fn ten_bit_notes() {
        let h = v(VideoCodec::Hevc);
        assert!(ten_bit_note(Some("yuv420p"), &h).unwrap().contains("8-bit"));
        assert!(ten_bit_note(Some("yuv420p10le"), &h).unwrap().contains("keeps"));
        assert_eq!(ten_bit_note(Some("yuv420p"), &VideoEncode::default()), None);
        assert_eq!(ten_bit_note(None, &h), None);
    }

    #[test]
    fn range_filter_stays_identical_for_h264_and_8bit() {
        use super::super::common::RANGE_FILTER;
        assert_eq!(range_filter(&VideoEncode::default()), RANGE_FILTER);
        let mut h = v(VideoCodec::Hevc);
        h.ten_bit = false;
        assert_eq!(range_filter(&h), RANGE_FILTER);
        let mut a = v(VideoCodec::Av1);
        a.ten_bit = false;
        assert_eq!(range_filter(&a), RANGE_FILTER);
        // 10-bit HEVC/AV1: squeeze + 10-bit format in one step; default 10-bit is on
        let h10 = v(VideoCodec::Hevc);
        assert!(h10.ten_bit);
        assert_eq!(range_filter(&h10), format!("{RANGE_FILTER},format=yuv420p10le"));
        assert_eq!(range_filter_pix(&h10, "yuv422p10le"), format!("{RANGE_FILTER},format=yuv422p10le"));
        let a10 = v(VideoCodec::Av1);
        assert_eq!(range_filter(&a10), format!("{RANGE_FILTER},format=yuv420p10le"));
    }


    #[test]
    fn encoder_runtime_uses_managed_when_current_lacks_av1() {
        use crate::ffmpeg::locate::{FfPaths, FfSource};
        let own = FfPaths { ffmpeg: "D:\\Tools\\FFmpeg\\bin\\ffmpeg.exe".into(), ffprobe: "D:\\Tools\\FFmpeg\\bin\\ffprobe.exe".into(), bin_dir: "D:\\Tools\\FFmpeg\\bin".into(), source: FfSource::Configured };
        let thin = Encoders { libx265: false, libsvtav1: false, ac3: false };
        let full = Encoders { libx265: true, libsvtav1: true, ac3: true };
        assert_eq!(
            encoder_runtime(&["libsvtav1"], Some(&own), Some(&thin), Some(&full), true),
            EncoderRuntime::UseManaged { short: "AV1" }
        );
        assert_eq!(
            encoder_runtime(&["libsvtav1"], Some(&own), Some(&thin), None, false),
            EncoderRuntime::NeedDownload { short: "AV1" }
        );
        assert_eq!(encoder_runtime(&["libsvtav1"], Some(&own), Some(&full), Some(&full), true), EncoderRuntime::Current);
        assert_eq!(download_prompt("AV1"), "Your FFmpeg doesn't support AV1. Download the current FFmpeg build?");
        assert_eq!(
            encoder_runtime(&["libsvtav1", "ac3"], Some(&own), Some(&thin), None, false),
            EncoderRuntime::NeedDownload { short: "AV1 or AC3" }
        );
    }

    #[test]
    fn encoders_in_args_finds_svt_and_ac3() {
        let a = ["-c:v".into(), "libsvtav1".into(), "-crf".into(), "28".into(), "-c:a".into(), "ac3".into()];
        assert_eq!(encoders_in_args(&a), ["libsvtav1", "ac3"]);
        assert!(encoders_in_args(&["-c:v".into(), "libx264".into(), "-c:a".into(), "aac".into()]).is_empty());
        let v = VideoEncode { codec: VideoCodec::Av1, ..Default::default() };
        let au = AudioEncode { codec: AudioCodec::Ac3, ..Default::default() };
        assert_eq!(needed_encoders(Some(&v), Some(&au)), ["libsvtav1", "ac3"]);
        assert!(needed_encoders(Some(&VideoEncode::default()), Some(&AudioEncode::default())).is_empty());
    }
}

