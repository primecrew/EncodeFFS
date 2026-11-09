//! Create Slideshow: still images → H.264 MP4 with optional audio and crossfades.
//! Pure command builder (no I/O). Duration math and xfade filter-graph offsets are unit-tested.

use std::path::{Path, PathBuf};

use super::common::*;
use super::{p, ArgVec, BuiltCommand};
use crate::util::fmt_num;

/// Supported still-image extensions (lower-case, no dot).
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp", "webp", "tif", "tiff"];

pub fn is_image(path: &Path) -> bool {
    IMAGE_EXTS.iter().any(|e| crate::util::ext_lower(path) == *e)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DurationMode {
    /// User sets seconds each image is shown (default).
    #[default]
    PerImage,
    /// User sets the total length; per-image time is computed.
    FixedTotal,
    /// Total length = probed audio duration (requires an audio file).
    MatchAudio,
}

impl DurationMode {
    pub const ALL: [DurationMode; 3] = [DurationMode::PerImage, DurationMode::FixedTotal, DurationMode::MatchAudio];
    pub fn label(self) -> &'static str {
        match self {
            DurationMode::PerImage => "Per image length",
            DurationMode::FixedTotal => "Fixed total length",
            DurationMode::MatchAudio => "Match audio length",
        }
    }
}

/// Crossfade / wipe transitions that need the FFmpeg `xfade` filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    /// Hard cut (always available; used when xfade is missing).
    None,
    /// True crossfade (default when xfade is available).
    #[default]
    Fade,
    FadeBlack,
    FadeWhite,
    Dissolve,
    WipeLeft,
    WipeRight,
    SlideLeft,
    SlideRight,
}

impl Transition {
    pub const ALL: [Transition; 9] = [
        Transition::None, Transition::Fade, Transition::FadeBlack, Transition::FadeWhite,
        Transition::Dissolve, Transition::WipeLeft, Transition::WipeRight, Transition::SlideLeft, Transition::SlideRight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Transition::None => "None (hard cut)",
            Transition::Fade => "Fade (crossfade)",
            Transition::FadeBlack => "Fade through black",
            Transition::FadeWhite => "Fade through white",
            Transition::Dissolve => "Dissolve",
            Transition::WipeLeft => "Wipe left",
            Transition::WipeRight => "Wipe right",
            Transition::SlideLeft => "Slide left",
            Transition::SlideRight => "Slide right",
        }
    }
    /// FFmpeg `xfade=transition=` name. None = hard cut / concat.
    pub fn xfade_name(self) -> Option<&'static str> {
        match self {
            Transition::None => None,
            Transition::Fade => Some("fade"),
            Transition::FadeBlack => Some("fadeblack"),
            Transition::FadeWhite => Some("fadewhite"),
            Transition::Dissolve => Some("dissolve"),
            Transition::WipeLeft => Some("wipeleft"),
            Transition::WipeRight => Some("wiperight"),
            Transition::SlideLeft => Some("slideleft"),
            Transition::SlideRight => Some("slideright"),
        }
    }
    pub fn needs_xfade(self) -> bool {
        self.xfade_name().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputSize {
    Auto,
    /// 16:9 Full HD (recommended default).
    #[default]
    S1920x1080,
    /// 15:8 widescreen (user-requested).
    S1920x1024,
    /// 16:9 HD.
    S1280x720,
    /// 4:3 XGA.
    S1024x768,
    /// 4:3 SVGA.
    S800x600,
    /// 4:3 VGA.
    S640x480,
}

impl OutputSize {
    pub const ALL: [OutputSize; 7] = [
        OutputSize::Auto, OutputSize::S1920x1080, OutputSize::S1920x1024, OutputSize::S1280x720,
        OutputSize::S1024x768, OutputSize::S800x600, OutputSize::S640x480,
    ];
    pub fn label(self) -> &'static str {
        match self {
            OutputSize::Auto => "Auto (from images)",
            OutputSize::S1920x1080 => "1920×1080 (16:9 Full HD)",
            OutputSize::S1920x1024 => "1920×1024 (15:8)",
            OutputSize::S1280x720 => "1280×720 (16:9 HD)",
            OutputSize::S1024x768 => "1024×768 (4:3)",
            OutputSize::S800x600 => "800×600 (4:3)",
            OutputSize::S640x480 => "640×480 (4:3)",
        }
    }
    pub fn dims(self) -> Option<(u32, u32)> {
        match self {
            OutputSize::Auto => None,
            OutputSize::S1920x1080 => Some((1920, 1080)),
            OutputSize::S1920x1024 => Some((1920, 1024)),
            OutputSize::S1280x720 => Some((1280, 720)),
            OutputSize::S1024x768 => Some((1024, 768)),
            OutputSize::S800x600 => Some((800, 600)),
            OutputSize::S640x480 => Some((640, 480)),
        }
    }
}

/// How to place an image that is smaller than the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SmallImage {
    /// Center on a black canvas (default — no unexpected softening).
    #[default]
    BlackBorder,
    /// Bicubic upscale to fit, then pad if needed.
    Upscale,
}

impl SmallImage {
    pub const ALL: [SmallImage; 2] = [SmallImage::BlackBorder, SmallImage::Upscale];
    pub fn label(self) -> &'static str {
        match self {
            SmallImage::BlackBorder => "Black border (no upscale)",
            SmallImage::Upscale => "Bicubic upscale to fit",
        }
    }
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct ImageInput {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlideshowParams {
    pub images: Vec<ImageInput>,
    pub audio: Option<PathBuf>,
    /// Probed audio duration when MatchAudio (or used as a hint).
    pub audio_duration_s: Option<f64>,
    pub output: PathBuf,
    pub duration_mode: DurationMode,
    /// Seconds each image is shown (PerImage).
    pub per_image_s: f64,
    /// Total length (FixedTotal).
    pub fixed_total_s: f64,
    pub transition: Transition,
    pub fade_s: f64,
    /// False when the FFmpeg build has no xfade filter — transitions that need it fall back to cuts.
    pub has_xfade: bool,
    pub size: OutputSize,
    pub small: SmallImage,
    pub fps: u32,
    pub video: VideoEncode,
    pub audio_bitrate_k: u32,
    pub faststart: bool,
}

/// Even canvas size for Auto: max width and max height among images, each rounded up to even, capped.
pub fn auto_size(images: &[ImageInput]) -> Result<(u32, u32), String> {
    if images.is_empty() {
        return Err("Add at least one image.".into());
    }
    let mut w = 0u32;
    let mut h = 0u32;
    for im in images {
        if im.width == 0 || im.height == 0 {
            return Err(format!("{}: invalid size {}×{}", im.path.display(), im.width, im.height));
        }
        w = w.max(im.width);
        h = h.max(im.height);
    }
    // yuv420p needs even dimensions; clamp absurd sizes
    w = ((w + 1) & !1).min(7680);
    h = ((h + 1) & !1).min(4320);
    if w < 2 || h < 2 {
        return Err("Auto size ended up too small.".into());
    }
    Ok((w, h))
}

pub fn canvas_size(pr: &SlideshowParams) -> Result<(u32, u32), String> {
    match pr.size.dims() {
        Some((w, h)) => Ok((w, h)),
        None => auto_size(&pr.images),
    }
}

/// Per-image display time and total length, accounting for crossfade overlap.
/// `total = N*D − (N−1)*fade` when N≥1 and a fade is used; one image → just D (no fade).
#[derive(Debug, Clone, PartialEq)]
pub struct Timing {
    pub per_image_s: f64,
    pub fade_s: f64,
    pub total_s: f64,
    /// Effective transition (None when N<2 or fade≈0 or xfade unavailable).
    pub transition: Transition,
}

pub fn timing(pr: &SlideshowParams) -> Result<Timing, String> {
    let n = pr.images.len();
    if n == 0 {
        return Err("Add at least one image.".into());
    }
    let mut fade = if n < 2 || pr.transition == Transition::None || pr.fade_s <= 0.0 {
        0.0
    } else {
        pr.fade_s
    };
    // without xfade we can only hard-cut
    let transition = if fade > 0.0 && pr.transition.needs_xfade() && !pr.has_xfade {
        Transition::None
    } else if fade <= 0.0 {
        Transition::None
    } else {
        pr.transition
    };
    if transition == Transition::None {
        fade = 0.0;
    }

    let (d, total) = match pr.duration_mode {
        DurationMode::PerImage => {
            if !(pr.per_image_s.is_finite() && pr.per_image_s > 0.0) {
                return Err("Per-image length must be a positive number of seconds.".into());
            }
            let d = pr.per_image_s;
            let total = d * n as f64 - fade * (n.saturating_sub(1) as f64);
            (d, total)
        }
        DurationMode::FixedTotal => {
            if !(pr.fixed_total_s.is_finite() && pr.fixed_total_s > 0.0) {
                return Err("Fixed total length must be a positive number of seconds.".into());
            }
            let total = pr.fixed_total_s;
            let d = (total + fade * (n.saturating_sub(1) as f64)) / n as f64;
            (d, total)
        }
        DurationMode::MatchAudio => {
            let Some(ad) = pr.audio_duration_s.filter(|d| d.is_finite() && *d > 0.0) else {
                return Err("Match audio length needs an audio file whose length can be read.".into());
            };
            if pr.audio.is_none() {
                return Err("Match audio length needs an audio file.".into());
            }
            let total = ad;
            let d = (total + fade * (n.saturating_sub(1) as f64)) / n as f64;
            (d, total)
        }
    };
    if !d.is_finite() || d <= 0.0 {
        return Err("Computed per-image length is not valid.".into());
    }
    if fade > 0.0 && fade >= d {
        return Err(format!(
            "Crossfade ({fade} s) must be shorter than each image's display time ({d:.3} s)."
        ));
    }
    if !(total.is_finite() && total > 0.0) {
        return Err("Computed total length is not valid.".into());
    }
    Ok(Timing { per_image_s: d, fade_s: fade, total_s: total, transition })
}

/// Scale/pad filter for one still → canvas. Bicubic; never stretches.
pub fn place_filter(iw: u32, ih: u32, cw: u32, ch: u32, small: SmallImage) -> String {
    let fits = iw <= cw && ih <= ch;
    let scale = if fits && small == SmallImage::BlackBorder {
        // no upscale: keep native size, then pad
        format!("scale={iw}:{ih}:flags=bicubic")
    } else {
        // fit inside canvas, preserve aspect
        format!("scale={cw}:{ch}:force_original_aspect_ratio=decrease:flags=bicubic")
    };
    format!("{scale},pad={cw}:{ch}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,format=yuv420p,fps=FPS_PLACEHOLDER")
}

fn place_filter_fps(iw: u32, ih: u32, cw: u32, ch: u32, small: SmallImage, fps: u32) -> String {
    place_filter(iw, ih, cw, ch, small).replace("FPS_PLACEHOLDER", &fps.to_string())
}

/// xfade offset for the transition after image `k` (0-based), given equal per-image duration.
pub fn xfade_offset(k: usize, per_image: f64, fade: f64) -> f64 {
    // offset_k = sum_{i=0..=k} D − (k+1)*F = (k+1)*D − (k+1)*F = (k+1)*(D−F)
    (k as f64 + 1.0) * (per_image - fade)
}

pub fn build(pr: &SlideshowParams) -> Result<BuiltCommand, String> {
    let t = timing(pr)?;
    let (cw, ch) = canvas_size(pr)?;
    let fps = if pr.fps == 0 { 30 } else { pr.fps };
    let n = pr.images.len();
    let mut a: Vec<String> = Vec::new();
    a.push("-y".into());
    for im in &pr.images {
        // -loop 1 -t D makes a video of exact length D from the still
        a.push_all(["-loop", "1", "-t"]);
        a.push(fmt_num(t.per_image_s));
        a.push("-i".into());
        a.push(p(&im.path));
    }
    let audio_idx = if let Some(aud) = &pr.audio {
        a.push("-i".into());
        a.push(p(aud));
        Some(n) // index after all images
    } else {
        None
    };

    // per-image prep filters → [v0]..[vN-1]
    let mut filter_parts: Vec<String> = Vec::new();
    for (i, im) in pr.images.iter().enumerate() {
        let f = place_filter_fps(im.width, im.height, cw, ch, pr.small, fps);
        filter_parts.push(format!("[{i}:v]{f}[v{i}]"));
    }

    let video_map = if n == 1 || t.transition == Transition::None || t.fade_s <= 0.0 {
        // hard cuts via concat
        let pads: String = (0..n).map(|i| format!("[v{i}]")).collect();
        filter_parts.push(format!("{pads}concat=n={n}:v=1:a=0[vout]"));
        "[vout]".to_string()
    } else {
        let name = t.transition.xfade_name().unwrap(); // guarded above
        let mut prev = format!("[v0]");
        for k in 0..n - 1 {
            let out = if k + 1 == n - 1 { "[vout]".to_string() } else { format!("[x{k}]") };
            let off = xfade_offset(k, t.per_image_s, t.fade_s);
            filter_parts.push(format!(
                "{prev}[v{}]xfade=transition={name}:duration={}:offset={}{out}",
                k + 1,
                fmt_num(t.fade_s),
                fmt_num(off),
            ));
            prev = out;
        }
        "[vout]".to_string()
    };

    a.push("-filter_complex".into());
    a.push(filter_parts.join(";"));
    a.push_all(["-map", &video_map]);

    // encoding defaults: libx264, yuv420p, medium, CRF 18 (caller may override via pr.video)
    let mut v = pr.video.clone();
    // slideshows are stills: don't also append the optional output_fps field
    v.output_fps.clear();
    let mut venc = x264_args(&v, None, false);
    // x264_args already sets pix_fmt; ensure no stray -vf
    a.extend(venc.drain(..));
    // force -t to the computed total so audio can't overrun
    a.push_all(["-t".to_string(), fmt_num(t.total_s)]);

    if let Some(ai) = audio_idx {
        a.push_all(["-map".to_string(), format!("{ai}:a:0?")]);
        a.push_all(["-c:a".to_string(), "aac".into(), "-b:a".into(), format!("{}k", pr.audio_bitrate_k)]);
        a.push("-shortest".into());
    } else {
        a.push_all(["-an"]);
    }
    if pr.faststart {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));

    let mut warnings = vec![];
    if pr.transition.needs_xfade() && !pr.has_xfade {
        warnings.push("This FFmpeg build has no xfade filter, so transitions fall back to hard cuts. Install a full GPL build (Help › Check for FFmpeg updates…) for crossfades.".into());
    }
    if n == 1 && pr.transition.needs_xfade() {
        warnings.push("One image: no between-image transition is applied.".into());
    }
    Ok(BuiltCommand { args: a, warnings, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(name: &str, w: u32, h: u32) -> ImageInput {
        ImageInput { path: PathBuf::from(name), width: w, height: h }
    }
    fn base(n: usize) -> SlideshowParams {
        SlideshowParams {
            images: (0..n).map(|i| img(&format!("/i/{i}.jpg"), 1920, 1080)).collect(),
            audio: None,
            audio_duration_s: None,
            output: "/o/show.mp4".into(),
            duration_mode: DurationMode::PerImage,
            per_image_s: 5.0,
            fixed_total_s: 60.0,
            transition: Transition::Fade,
            fade_s: 1.0,
            has_xfade: true,
            size: OutputSize::S1920x1080,
            small: SmallImage::BlackBorder,
            fps: 30,
            video: VideoEncode { crf: 18, preset: "medium".into(), ..Default::default() },
            audio_bitrate_k: 192,
            faststart: true,
        }
    }

    #[test]
    fn duration_math_per_image_with_and_without_fade() {
        let mut pr = base(3);
        let t = timing(&pr).unwrap();
        assert!((t.per_image_s - 5.0).abs() < 1e-9);
        assert!((t.total_s - (3.0 * 5.0 - 2.0 * 1.0)).abs() < 1e-9); // 13
        pr.transition = Transition::None;
        let t = timing(&pr).unwrap();
        assert!((t.total_s - 15.0).abs() < 1e-9);
        pr.transition = Transition::Fade;
        pr.images = vec![img("/i/0.jpg", 100, 100)];
        let t = timing(&pr).unwrap();
        assert_eq!(t.transition, Transition::None);
        assert!((t.total_s - 5.0).abs() < 1e-9);
    }

    #[test]
    fn fixed_total_and_match_audio() {
        let mut pr = base(4);
        pr.duration_mode = DurationMode::FixedTotal;
        pr.fixed_total_s = 40.0;
        pr.fade_s = 1.0;
        let t = timing(&pr).unwrap();
        // 40 = 4*D - 3*1 → D = 43/4 = 10.75
        assert!((t.per_image_s - 10.75).abs() < 1e-9);
        assert!((t.total_s - 40.0).abs() < 1e-9);

        pr.duration_mode = DurationMode::MatchAudio;
        pr.audio = Some("/a/song.wav".into());
        pr.audio_duration_s = Some(40.0);
        let t = timing(&pr).unwrap();
        assert!((t.per_image_s - 10.75).abs() < 1e-9);

        pr.audio = None;
        assert!(timing(&pr).unwrap_err().contains("audio"));
        pr.audio = Some("/a/song.wav".into());
        pr.audio_duration_s = None;
        assert!(timing(&pr).unwrap_err().contains("audio"));
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut pr = base(2);
        pr.per_image_s = 0.0;
        assert!(timing(&pr).is_err());
        pr.per_image_s = -1.0;
        assert!(timing(&pr).is_err());
        pr.per_image_s = f64::NAN;
        assert!(timing(&pr).is_err());
        pr.per_image_s = 1.0;
        pr.fade_s = 1.0; // fade >= D
        assert!(timing(&pr).unwrap_err().contains("shorter"));
        pr.images.clear();
        assert!(timing(&pr).unwrap_err().contains("one image"));
    }

    #[test]
    fn xfade_offsets() {
        // D=5, F=1 → offsets 4, 8, 12…
        assert!((xfade_offset(0, 5.0, 1.0) - 4.0).abs() < 1e-9);
        assert!((xfade_offset(1, 5.0, 1.0) - 8.0).abs() < 1e-9);
        assert!((xfade_offset(2, 5.0, 1.0) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn missing_xfade_falls_back_to_cuts() {
        let mut pr = base(3);
        pr.has_xfade = false;
        pr.transition = Transition::Fade;
        let t = timing(&pr).unwrap();
        assert_eq!(t.transition, Transition::None);
        assert_eq!(t.fade_s, 0.0);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("concat=n=3"), "{s}");
        assert!(!s.contains("xfade="), "{s}");
        assert!(b.warnings.iter().any(|w| w.contains("xfade")));
    }

    #[test]
    fn with_xfade_builds_chain() {
        let pr = base(3);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("xfade=transition=fade:duration=1:offset=4"), "{s}");
        assert!(s.contains("xfade=transition=fade:duration=1:offset=8"), "{s}");
        assert!(s.contains("-c:v libx264 -crf 18 -preset medium -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("-movflags +faststart"), "{s}");
        assert!(s.contains("-an "), "{s}");
        assert!(!s.contains(" -r "), "must not use -r (stretches duration)");
        // three looped inputs
        assert_eq!(s.matches("-loop 1 -t 5").count(), 3);
    }

    #[test]
    fn with_audio_maps_aac() {
        let mut pr = base(2);
        pr.audio = Some("/a/voice.wav".into());
        pr.audio_duration_s = Some(9.0);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-i /a/voice.wav") && s.contains("-c:a aac -b:a 192k") && s.contains("-shortest"), "{s}");
    }

    #[test]
    fn auto_even_size_and_place_filter() {
        let images = vec![img("a.png", 1001, 500), img("b.png", 800, 601)];
        assert_eq!(auto_size(&images).unwrap(), (1002, 602));
        // small image, black border: no upscale
        let f = place_filter(400, 300, 1920, 1080, SmallImage::BlackBorder);
        assert!(f.contains("scale=400:300") && f.contains("pad=1920:1080"), "{f}");
        // small image, upscale
        let f = place_filter(400, 300, 1920, 1080, SmallImage::Upscale);
        assert!(f.contains("force_original_aspect_ratio=decrease") && f.contains("flags=bicubic"), "{f}");
        // large image always fits inside
        let f = place_filter(4000, 2000, 1280, 720, SmallImage::BlackBorder);
        assert!(f.contains("force_original_aspect_ratio=decrease"), "{f}");
    }

    #[test]
    fn single_image_no_xfade() {
        let pr = base(1);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("concat=n=1") || s.contains("[0:v]"), "{s}");
        assert!(!s.contains("xfade="), "{s}");
    }

    #[test]
    fn is_image_exts() {
        assert!(is_image(Path::new("a.JPG")));
        assert!(is_image(Path::new("b.webp")));
        assert!(!is_image(Path::new("c.gif")));
        assert!(!is_image(Path::new("d.mp4")));
    }
}
