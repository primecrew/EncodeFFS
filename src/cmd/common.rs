//! Building blocks shared by several modes: x264/AAC settings, audio gain +
//! compressor chains, colour-range conversion, deinterlacing, resize maths,
//! passthrough and MP4 subtitle compatibility.

use serde::{Deserialize, Serialize};

use super::ArgVec;
use crate::util::fmt_num;

/// x264 presets, index 0..=8 corresponds to the script's 1..9 menu.
pub const PRESETS: [&str; 9] = [
    "ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow",
];
/// x264 tunes offered by the script.
pub const TUNES: [&str; 5] = ["film", "animation", "grain", "fastdecode", "zerolatency"];

/// Full-range -> limited-range conversion filter (script: `scale=in_range=full:out_range=tv`).
pub const RANGE_FILTER: &str = "scale=in_range=full:out_range=tv";
/// bwdif double-rate deinterlace used by the script.
pub const DEINTERLACE_FILTER: &str = "bwdif=mode=send_field:parity=auto:deint=interlaced";

/// BT.709 limited-range tags (script: Get-ColorRangeMetadataArgs).
pub fn color_tag_args() -> Vec<String> {
    ["-color_range", "tv", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Which streams are copied instead of re-encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Passthrough {
    /// Re-encode video and audio.
    #[default]
    None,
    /// Copy audio, re-encode video.
    Audio,
    /// Copy video, re-encode audio.
    Video,
    /// Copy both.
    Both,
}

impl Passthrough {
    pub const ALL: [Passthrough; 4] = [Passthrough::None, Passthrough::Audio, Passthrough::Video, Passthrough::Both];
    pub fn encodes_video(self) -> bool {
        matches!(self, Passthrough::None | Passthrough::Audio)
    }
    pub fn encodes_audio(self) -> bool {
        matches!(self, Passthrough::None | Passthrough::Video)
    }
    pub fn label(self) -> &'static str {
        match self {
            Passthrough::None => "None (re-encode video + audio)",
            Passthrough::Audio => "Audio (copy audio, re-encode video)",
            Passthrough::Video => "Video (copy video, re-encode audio)",
            Passthrough::Both => "Both (copy video + audio)",
        }
    }
}

/// x264 settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoEncode {
    pub crf: u32,
    pub preset: String,
    /// None = no -tune.
    pub tune: Option<String>,
    /// Codec (codecs branch). Missing in older presets = H.264, so they build the same command.
    pub codec: super::codecs::VideoCodec,
    /// HEVC/AV1: write 10-bit (yuv420p10le). Ignored for H.264.
    pub ten_bit: bool,
    /// AV1 (SVT-AV1) preset 0–13.
    pub svt_preset: u32,
    /// AV1 film grain synthesis 0 (off) – 50.
    pub film_grain: u32,
    pub film_grain_denoise: bool,
    /// Output frame rate ("" = same as source, the default). A positive number ("5", "29.97") or a
    /// fraction ("30000/1001"). Added as the `fps` filter at the end of the video filter chain, so
    /// FFmpeg drops or duplicates frames: duration, audio and sync stay the same (no speed change).
    pub output_fps: String,
}
impl Default for VideoEncode {
    fn default() -> Self {
        VideoEncode { crf: 22, preset: "slow".into(), tune: None, codec: Default::default(), ten_bit: true, svt_preset: 5, film_grain: 0, film_grain_denoise: false, output_fps: String::new() }
    }
}

/// Highest output frame rate accepted.
pub const MAX_OUTPUT_FPS: f64 = 1000.0;

/// Check an output frame rate: Ok(None) = same as source, Ok(Some(rate)) = the trimmed rate to pass
/// to FFmpeg, Err = a plain message (zero, negative, NaN, infinite, not a number, …).
pub fn parse_output_fps(s: &str) -> Result<Option<String>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let bad = || format!("Output frame rate “{t}” is not valid. Use a positive number like 5, 29.97 or 30000/1001, or leave it empty for the same as the source.");
    let value = if let Some((n, d)) = t.split_once('/') {
        let (n, d) = (n.trim().parse::<u64>().map_err(|_| bad())?, d.trim().parse::<u64>().map_err(|_| bad())?);
        if n == 0 || d == 0 {
            return Err(bad());
        }
        n as f64 / d as f64
    } else {
        if !t.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err(bad()); // no signs, exponents, "inf" or "NaN"
        }
        t.parse::<f64>().map_err(|_| bad())?
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(bad());
    }
    if value > MAX_OUTPUT_FPS {
        return Err(format!("Output frame rate {t} is too high (at most {MAX_OUTPUT_FPS} fps)."));
    }
    Ok(Some(t.replace(' ', "")))
}

/// The video filter chain with `fps=RATE` appended when an output frame rate is set (unchanged for
/// "same as source" or an invalid value, which the forms reject before building).
pub fn with_output_fps(filter: Option<&str>, v: &VideoEncode) -> Option<String> {
    let base = filter.filter(|f| !f.is_empty());
    match parse_output_fps(&v.output_fps) {
        Ok(Some(r)) => Some(match base {
            Some(f) => format!("{f},fps={r}"),
            None => format!("fps={r}"),
        }),
        _ => base.map(str::to_string),
    }
}

/// `-c:v libx264 -crf N -preset P -pix_fmt yuv420p [-vf F] [range tags] [-tune T]`
pub fn x264_args(v: &VideoEncode, filter: Option<&str>, range_tags: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    a.push_all(["-c:v", "libx264", "-crf"]);
    a.push(v.crf.to_string());
    a.push("-preset".into());
    a.push(v.preset.clone());
    a.push_all(["-pix_fmt", "yuv420p"]);
    if let Some(f) = with_output_fps(filter, v) {
        a.push("-vf".into());
        a.push(f);
    }
    if range_tags {
        a.extend(color_tag_args());
    }
    if let Some(t) = v.tune.as_ref().filter(|t| !t.is_empty()) {
        a.push_all(["-tune", t.as_str()]);
    }
    a
}

/// Compressor presets offered by the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompressorPreset {
    #[default]
    None,
    Light,
    Medium,
    Strong,
    Advanced,
}
impl CompressorPreset {
    pub const ALL: [CompressorPreset; 5] = [
        CompressorPreset::None,
        CompressorPreset::Light,
        CompressorPreset::Medium,
        CompressorPreset::Strong,
        CompressorPreset::Advanced,
    ];
    pub fn label(self) -> &'static str {
        match self {
            CompressorPreset::None => "none",
            CompressorPreset::Light => "light",
            CompressorPreset::Medium => "medium",
            CompressorPreset::Strong => "strong",
            CompressorPreset::Advanced => "advanced",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            CompressorPreset::None => "no compression, just use the gain",
            CompressorPreset::Light => "gentle control for a small boost",
            CompressorPreset::Medium => "balanced default for most speech and mixed content",
            CompressorPreset::Strong => "firmer peak control when pushing audio harder",
            CompressorPreset::Advanced => "enter threshold, ratio, knee, makeup, attack, release",
        }
    }
}

/// acompressor parameters in user units (dB / ms).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressorParams {
    pub threshold_db: f64,
    pub ratio: f64,
    pub knee: f64,
    pub makeup_db: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
}
impl Default for CompressorParams {
    fn default() -> Self {
        // Script defaults ($defaultCompression*)
        CompressorParams { threshold_db: -18.0, ratio: 2.0, knee: 4.0, makeup_db: 2.0, attack_ms: 20.0, release_ms: 250.0 }
    }
}

impl CompressorParams {
    /// Resolve a preset to concrete parameters (`advanced` returns the user's values).
    pub fn for_preset(preset: CompressorPreset, advanced: &CompressorParams) -> Option<CompressorParams> {
        match preset {
            CompressorPreset::None => None,
            CompressorPreset::Light => Some(CompressorParams { threshold_db: -16.0, ratio: 1.5, knee: 6.0, makeup_db: 1.0, attack_ms: 25.0, release_ms: 300.0 }),
            CompressorPreset::Medium => Some(CompressorParams { threshold_db: -18.0, ratio: 2.0, knee: 4.0, makeup_db: 2.0, attack_ms: 20.0, release_ms: 250.0 }),
            CompressorPreset::Strong => Some(CompressorParams { threshold_db: -20.0, ratio: 3.0, knee: 3.0, makeup_db: 2.0, attack_ms: 10.0, release_ms: 180.0 }),
            CompressorPreset::Advanced => Some(*advanced),
        }
    }

    /// Same limits as the script's advanced prompts.
    pub fn validate(&self) -> Result<(), String> {
        if !(-60.0..=0.0).contains(&self.threshold_db) {
            return Err(format!("Invalid compression threshold: {} dB (must be -60..0)", self.threshold_db));
        }
        if !(1.0..=20.0).contains(&self.ratio) {
            return Err(format!("Invalid compression ratio: {} (must be 1..20)", self.ratio));
        }
        if !(1.0..=8.0).contains(&self.knee) {
            return Err(format!("Invalid knee value: {} (must be 1..8)", self.knee));
        }
        if !(-12.0..=12.0).contains(&self.makeup_db) {
            return Err(format!("Invalid makeup gain: {} dB (must be -12..12)", self.makeup_db));
        }
        if !(0.01..=2000.0).contains(&self.attack_ms) {
            return Err(format!("Invalid attack time: {} ms (must be 0.01..2000)", self.attack_ms));
        }
        if !(1.0..=9000.0).contains(&self.release_ms) {
            return Err(format!("Invalid release time: {} ms (must be 1..9000)", self.release_ms));
        }
        Ok(())
    }

    /// The acompressor filter string, formatted exactly like the script.
    pub fn filter(&self) -> String {
        format!(
            "acompressor=mode=downward:threshold={}:ratio={}:knee={}:attack={}:release={}:makeup={}",
            fmt_num(db_to_linear(self.threshold_db)),
            fmt_num(self.ratio),
            fmt_num(self.knee),
            fmt_num(self.attack_ms),
            fmt_num(self.release_ms),
            fmt_num(db_to_linear(self.makeup_db)),
        )
    }
}

pub fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// AAC settings plus optional gain/compressor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioEncode {
    pub bitrate_k: u32,
    pub gain_db: f64,
    pub compressor: CompressorPreset,
    /// Used when `compressor == Advanced`.
    pub advanced: CompressorParams,
    /// Codec (codecs branch). Missing in older presets = AAC.
    pub codec: super::codecs::AudioCodec,
    /// AC3 bitrate (kbps).
    pub ac3_bitrate_k: u32,
    /// Most channels in any source audio track (from ffprobe; not saved). AC3 downmixes > 5.1.
    #[serde(skip)]
    pub source_channels: Option<u32>,
}
impl Default for AudioEncode {
    fn default() -> Self {
        AudioEncode { bitrate_k: 192, gain_db: 0.0, compressor: CompressorPreset::None, advanced: CompressorParams::default(), codec: Default::default(), ac3_bitrate_k: 448, source_channels: None }
    }
}

impl AudioEncode {
    pub fn validate(&self) -> Result<(), String> {
        if !(-30.0..=30.0).contains(&self.gain_db) {
            return Err(format!("Invalid audio gain: {} (must be -30..+30 dB)", self.gain_db));
        }
        if self.bitrate_k == 0 {
            return Err("Audio bitrate must be > 0".into());
        }
        if let Some(c) = CompressorParams::for_preset(self.compressor, &self.advanced) {
            c.validate()?;
        }
        Ok(())
    }

    /// `volume=XdB[,acompressor=...]` or None when nothing to do.
    pub fn filter_chain(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.gain_db != 0.0 {
            parts.push(format!("volume={}dB", fmt_num(self.gain_db)));
        }
        if let Some(c) = CompressorParams::for_preset(self.compressor, &self.advanced) {
            parts.push(c.filter());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(","))
        }
    }

    /// `-c:a aac -b:a Nk`
    pub fn aac_args(&self) -> Vec<String> {
        vec!["-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", self.bitrate_k)]
    }
}

/// Join non-empty filter parts with ','.
pub fn join_filters(parts: &[Option<String>]) -> Option<String> {
    let v: Vec<&str> = parts.iter().flatten().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
    if v.is_empty() {
        None
    } else {
        Some(v.join(","))
    }
}

/// Resize keeping aspect ratio with even dimensions, exactly like the script:
/// odd target height is bumped +1, width = Round(h * w/h) (banker's rounding like
/// .NET Math.Round), bumped +1 if odd.
pub fn resize_dims(width: u32, height: u32, target_height: u32) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 {
        return Err(format!("Invalid source resolution {}x{}", width, height));
    }
    if !(1..=4320).contains(&target_height) {
        return Err(format!("Invalid vertical resolution: {} (1..4320)", target_height));
    }
    let mut h = target_height;
    if h % 2 != 0 {
        h += 1;
    }
    let aspect = width as f64 / height as f64;
    let mut w = (h as f64 * aspect).round_ties_even() as u32;
    if w % 2 != 0 {
        w += 1;
    }
    if w == 0 {
        return Err(format!("Invalid resolution: {}x{}", w, h));
    }
    Ok((w, h))
}

/// ffprobe `field_order` -> interlaced? (script: present and not progressive/unknown)
pub fn is_interlaced(field_order: Option<&str>) -> bool {
    match field_order {
        Some(f) => !f.is_empty() && f != "progressive" && f != "unknown",
        None => false,
    }
}

/// Subtitle codecs that can go into MP4 (converted to mov_text). Same list as the script.
pub const MP4_TEXT_SUB_CODECS: &[&str] = &["subrip", "srt", "ass", "ssa", "webvtt", "text", "mov_text", "tx3g"];

pub fn mp4_sub_compatible(codec: &str) -> bool {
    MP4_TEXT_SUB_CODECS.contains(&codec)
}

/// Faststart args (only meaningful for MP4/MOV outputs).
pub fn faststart_args() -> Vec<String> {
    vec!["-movflags".into(), "+faststart".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressor_presets_match_script() {
        let adv = CompressorParams::default();
        assert!(CompressorParams::for_preset(CompressorPreset::None, &adv).is_none());
        let m = CompressorParams::for_preset(CompressorPreset::Medium, &adv).unwrap();
        assert_eq!(
            m.filter(),
            "acompressor=mode=downward:threshold=0.125893:ratio=2:knee=4:attack=20:release=250:makeup=1.258925"
        );
        let l = CompressorParams::for_preset(CompressorPreset::Light, &adv).unwrap();
        assert_eq!(
            l.filter(),
            "acompressor=mode=downward:threshold=0.158489:ratio=1.5:knee=6:attack=25:release=300:makeup=1.122018"
        );
        let s = CompressorParams::for_preset(CompressorPreset::Strong, &adv).unwrap();
        assert_eq!(
            s.filter(),
            "acompressor=mode=downward:threshold=0.1:ratio=3:knee=3:attack=10:release=180:makeup=1.258925"
        );
    }

    #[test]
    fn compressor_validation_limits() {
        let mut c = CompressorParams::default();
        assert!(c.validate().is_ok());
        c.threshold_db = 1.0;
        assert!(c.validate().is_err());
        c = CompressorParams { ratio: 25.0, ..Default::default() };
        assert!(c.validate().is_err());
        c = CompressorParams { knee: 0.5, ..Default::default() };
        assert!(c.validate().is_err());
        c = CompressorParams { attack_ms: 0.0, ..Default::default() };
        assert!(c.validate().is_err());
        c = CompressorParams { release_ms: 9001.0, ..Default::default() };
        assert!(c.validate().is_err());
    }

    #[test]
    fn audio_chain() {
        let mut a = AudioEncode::default();
        assert_eq!(a.filter_chain(), None);
        a.gain_db = 3.5;
        assert_eq!(a.filter_chain().unwrap(), "volume=3.5dB");
        a.compressor = CompressorPreset::Strong;
        assert!(a.filter_chain().unwrap().starts_with("volume=3.5dB,acompressor=mode=downward:threshold=0.1:"));
        a.gain_db = -40.0;
        assert!(a.validate().is_err());
        assert_eq!(AudioEncode::default().aac_args(), vec!["-c:a", "aac", "-b:a", "192k"]);
    }

    #[test]
    fn resize_math() {
        assert_eq!(resize_dims(1920, 1080, 720).unwrap(), (1280, 720));
        assert_eq!(resize_dims(1920, 1080, 719).unwrap(), (1280, 720)); // odd height bumped
        assert_eq!(resize_dims(1366, 768, 720).unwrap(), (1282, 720)); // 1280.625 -> 1281 -> even 1282
        assert_eq!(resize_dims(720, 480, 1080).unwrap(), (1620, 1080));
        // banker's rounding: 4x3 -> h=2 gives 2.666.. -> 3 -> 4
        assert_eq!(resize_dims(4, 3, 2).unwrap(), (4, 2));
        // exact .5 tie rounds to even: w/h = 5/4, h=2 -> 2.5 -> 2
        assert_eq!(resize_dims(5, 4, 2).unwrap(), (2, 2));
        assert!(resize_dims(1920, 1080, 0).is_err());
        assert!(resize_dims(1920, 1080, 5000).is_err());
    }

    #[test]
    fn x264() {
        let v = VideoEncode { crf: 20, preset: "fast".into(), tune: Some("film".into()), ..Default::default() };
        assert_eq!(
            x264_args(&v, Some("bwdif"), true),
            vec!["-c:v", "libx264", "-crf", "20", "-preset", "fast", "-pix_fmt", "yuv420p", "-vf", "bwdif",
                 "-color_range", "tv", "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
                 "-tune", "film"]
        );
        let v = VideoEncode::default();
        assert_eq!(x264_args(&v, None, false), vec!["-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p"]);
    }

    #[test]
    fn interlace_flag() {
        assert!(is_interlaced(Some("tt")));
        assert!(is_interlaced(Some("tb")));
        assert!(!is_interlaced(Some("progressive")));
        assert!(!is_interlaced(Some("unknown")));
        assert!(!is_interlaced(None));
    }

    #[test]
    fn filters_join() {
        assert_eq!(join_filters(&[None, Some("".into())]), None);
        assert_eq!(join_filters(&[Some(DEINTERLACE_FILTER.into()), Some(RANGE_FILTER.into())]).unwrap(),
                   format!("{},{}", DEINTERLACE_FILTER, RANGE_FILTER));
    }

    #[test]
    fn mp4_subs() {
        assert!(mp4_sub_compatible("subrip"));
        assert!(mp4_sub_compatible("ass"));
        assert!(!mp4_sub_compatible("hdmv_pgs_subtitle"));
        assert!(!mp4_sub_compatible("dvd_subtitle"));
    }

    #[test]
    fn passthrough_flags() {
        assert!(Passthrough::None.encodes_video() && Passthrough::None.encodes_audio());
        assert!(Passthrough::Audio.encodes_video() && !Passthrough::Audio.encodes_audio());
        assert!(!Passthrough::Video.encodes_video() && Passthrough::Video.encodes_audio());
        assert!(!Passthrough::Both.encodes_video() && !Passthrough::Both.encodes_audio());
    }
}
