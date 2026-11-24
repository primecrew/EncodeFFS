//! Frameserving (DebugMode FrameServer and similar): an editor serves frames
//! through a small "signpost" .avi; an AviSynth script opens it with AviSource
//! and ffmpeg reads the script (`-i script.avs`, avisynth demuxer).
//!
//! Function names follow the AviSynth+ documentation (avisynth.nl "Convert",
//! "AviSource", "ConvertBits", "ChangeFPS", "Trim"). Strings in AviSynth have no
//! escape sequences, so a path is written as-is between quotes (Windows paths
//! can't contain `"` anyway; if one does, the triple-quote form is used).
//! Non-ASCII paths get `utf8=true` (AviSynth+ r2768 and newer); the script itself
//! is written as UTF-8 without BOM, which AviSynth+ accepts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::common::*;
use super::{p, ArgVec, BuiltCommand};

/// Prefix of the temporary scripts in %TEMP% (cleaned up on start-up if left over).
pub const TEMP_PREFIX: &str = "ffenc_frameserve_";

/// Colour format the script converts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Color {
    /// ConvertToYUV420() – AviSynth+ name, same 4:2:0 layout as YV12.
    #[default]
    Yuv420,
    /// ConvertToYV12() – classic 8-bit name of 4:2:0.
    Yv12,
    Yuv422,
    Yuv444,
    Rgb32,
    /// No conversion: whatever the signpost delivers.
    None,
}

impl Color {
    pub const ALL: [Color; 6] = [Color::Yuv420, Color::Yv12, Color::Yuv422, Color::Yuv444, Color::Rgb32, Color::None];
    /// AviSynth function, or None for "no conversion".
    pub fn function(self) -> Option<&'static str> {
        match self {
            Color::Yuv420 => Some("ConvertToYUV420"),
            Color::Yv12 => Some("ConvertToYV12"),
            Color::Yuv422 => Some("ConvertToYUV422"),
            Color::Yuv444 => Some("ConvertToYUV444"),
            Color::Rgb32 => Some("ConvertToRGB32"),
            Color::None => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Color::Yuv420 => "YUV 4:2:0 – ConvertToYUV420() (recommended)",
            Color::Yv12 => "YV12 – ConvertToYV12() (older name, 8-bit 4:2:0)",
            Color::Yuv422 => "YUV 4:2:2 – ConvertToYUV422()",
            Color::Yuv444 => "YUV 4:4:4 – ConvertToYUV444()",
            Color::Rgb32 => "RGB32 – ConvertToRGB32()",
            Color::None => "None – keep what the signpost gives",
        }
    }
    /// Plain-language help: what it is and when to pick it.
    pub fn help(self) -> &'static str {
        match self {
            Color::Yuv420 => "The normal choice. H.264/H.265 in MP4 or MKV uses this layout, and it plays everywhere (TVs, phones, browsers, YouTube).",
            Color::Yv12 => "The same 4:2:0 layout as YUV420 under its older 8-bit name. Only pick it for old AviSynth filters or plugins that ask for YV12. It can't be combined with 10-bit.",
            Color::Yuv422 => "Keeps twice the colour detail of 4:2:0. Good for editing or archiving, but files are larger and many players and devices can't play it.",
            Color::Yuv444 => "Keeps full colour detail (sharp coloured text and lines). For archiving or screen recordings; largest files and the least compatible.",
            Color::Rgb32 => "Only for AviSynth filters that need RGB. The encoder converts it back to YUV 4:2:0 for the video file anyway.",
            Color::None => "No conversion line. The script passes on whatever the signpost delivers (set by the frameserver's own output setting, often RGB or YUY2); FFmpeg converts it for the encoder.",
        }
    }
    /// Encoder pixel format for this layout.
    pub fn pix_fmt(self, bits: BitDepth) -> String {
        let base = match self {
            Color::Yuv422 => "yuv422p",
            Color::Yuv444 => "yuv444p",
            _ => "yuv420p",
        };
        match bits {
            BitDepth::Eight => base.into(),
            BitDepth::Ten => format!("{base}10le"),
        }
    }
}

/// Colour matrix for RGB <-> YUV conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Matrix {
    /// Don't write matrix= (AviSynth default Rec601).
    Default,
    #[default]
    Rec709,
    Rec601,
    Pc709,
    Pc601,
    Rec2020,
}

impl Matrix {
    pub const ALL: [Matrix; 6] = [Matrix::Pc709, Matrix::Rec709, Matrix::Rec601, Matrix::Pc601, Matrix::Rec2020, Matrix::Default];
    pub fn value(self) -> Option<&'static str> {
        match self {
            Matrix::Default => None,
            Matrix::Rec709 => Some("Rec709"),
            Matrix::Rec601 => Some("Rec601"),
            Matrix::Pc709 => Some("PC.709"),
            Matrix::Pc601 => Some("PC.601"),
            Matrix::Rec2020 => Some("Rec2020"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Matrix::Default => "Not set (AviSynth default, Rec601)",
            Matrix::Rec709 => "Rec709 – HD, squeeze to TV range (Vegas 32-bit full-range)",
            Matrix::Rec601 => "Rec601 – standard definition (DVD, 480/576 lines)",
            Matrix::Pc709 => "PC.709 – HD, keep full range (recommended for Vegas 8-bit)",
            Matrix::Pc601 => "PC.601 – SD, keep full range",
            Matrix::Rec2020 => "Rec2020 – UHD / HDR projects",
        }
    }
    /// Rec709 / Rec601 / Rec2020 already convert to TV (limited) range in the script.
    pub fn squeezes_to_tv(self) -> bool {
        matches!(self, Matrix::Rec709 | Matrix::Rec601 | Matrix::Rec2020)
    }
    /// PC.709 / PC.601 keep the editor's full range.
    pub fn keeps_full_range(self) -> bool {
        matches!(self, Matrix::Pc709 | Matrix::Pc601)
    }
    /// FFmpeg colour-range / colour-space tags. `squeezed_to_tv` is set when a PC matrix
    /// was then converted to TV range in FFmpeg (never when the script already squeezed).
    pub fn color_tag_args(self, squeezed_to_tv: bool) -> Vec<String> {
        let tags = |range: &str, space: &str, prim: &str, trc: &str| {
            ["-color_range", range, "-colorspace", space, "-color_primaries", prim, "-color_trc", trc]
                .iter()
                .map(|s| s.to_string())
                .collect()
        };
        match (self, squeezed_to_tv) {
            (Matrix::Rec709, _) | (Matrix::Pc709, true) => super::common::color_tag_args(),
            (Matrix::Pc709, false) => tags("pc", "bt709", "bt709", "bt709"),
            (Matrix::Rec601, _) | (Matrix::Pc601, true) => tags("tv", "smpte170m", "smpte170m", "smpte170m"),
            (Matrix::Pc601, false) => tags("pc", "smpte170m", "smpte170m", "smpte170m"),
            (Matrix::Rec2020, _) => tags("tv", "bt2020nc", "bt2020", "bt2020-10"),
            (Matrix::Default, _) => vec![],
        }
    }
}

/// When a saved preset omitted `matrix`, the effective value used to be Rec709.
pub fn missing_matrix() -> Matrix {
    Matrix::Rec709
}

/// AviSource pixel_type (what the decompressor is asked to deliver).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PixelType {
    #[default]
    Auto,
    Yv12,
    Yuy2,
    Rgb32,
    Rgb24,
}

impl PixelType {
    pub const ALL: [PixelType; 5] = [PixelType::Auto, PixelType::Yuy2, PixelType::Rgb32, PixelType::Rgb24, PixelType::Yv12];
    pub fn value(self) -> Option<&'static str> {
        match self {
            PixelType::Auto => None,
            PixelType::Yv12 => Some("YV12"),
            PixelType::Yuy2 => Some("YUY2"),
            PixelType::Rgb32 => Some("RGB32"),
            PixelType::Rgb24 => Some("RGB24"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PixelType::Auto => "Automatic (recommended)",
            PixelType::Yv12 => "YV12",
            PixelType::Yuy2 => "YUY2",
            PixelType::Rgb32 => "RGB32",
            PixelType::Rgb24 => "RGB24",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioMode {
    /// Keep the served audio.
    #[default]
    Keep,
    /// Keep it and convert to 16-bit (ConvertAudioTo16bit()).
    To16bit,
    /// No audio (audio=false).
    None,
}

impl AudioMode {
    pub const ALL: [AudioMode; 3] = [AudioMode::Keep, AudioMode::To16bit, AudioMode::None];
    pub fn label(self) -> &'static str {
        match self {
            AudioMode::Keep => "Keep the audio",
            AudioMode::To16bit => "Keep it, converted to 16-bit (ConvertAudioTo16bit) – try this if audio fails",
            AudioMode::None => "No audio (video only)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BitDepth {
    #[default]
    Eight,
    /// ConvertBits(10) + a 10-bit encoder pixel format.
    Ten,
}

/// Everything the script generator needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptOptions {
    pub signpost: PathBuf,
    pub pixel_type: PixelType,
    pub audio: AudioMode,
    pub color: Color,
    pub matrix: Matrix,
    pub interlaced: bool,
    /// (first frame, last frame; 0 = to the end).
    pub trim: Option<(u64, u64)>,
    /// Convert to this frame rate (numerator, denominator) with ChangeFPS: frames are repeated or
    /// dropped, so the length and the audio stay as they are (AssumeFPS would change the speed of
    /// the video only and put the audio out of sync).
    pub fps: Option<(u32, u32)>,
    pub bits: BitDepth,
}

impl ScriptOptions {
    pub fn new(signpost: impl Into<PathBuf>) -> Self {
        ScriptOptions {
            signpost: signpost.into(),
            pixel_type: PixelType::Auto,
            audio: AudioMode::Keep,
            color: Color::Yuv420,
            matrix: Matrix::Pc709,
            interlaced: false,
            trim: None,
            fps: None,
            bits: BitDepth::Eight,
        }
    }
}

/// An AviSynth string literal. AviSynth has no escapes inside "…", so text with
/// a `"` uses the triple-quoted form; text with `"""` or a line break can't be written.
pub fn quote(s: &str) -> Result<String, String> {
    if s.contains('\n') || s.contains('\r') {
        return Err("The path contains a line break.".into());
    }
    if !s.contains('"') {
        return Ok(format!("\"{s}\""));
    }
    if s.contains("\"\"\"") || s.ends_with('"') {
        return Err(format!("This path can't be written in an AviSynth script: {s}"));
    }
    Ok(format!("\"\"\"{s}\"\"\""))
}

/// Common frame rates offered in the form: (label, num, den).
pub const FPS_CHOICES: &[(&str, u32, u32)] = &[
    ("23.976", 24000, 1001),
    ("24", 24, 1),
    ("25", 25, 1),
    ("29.97", 30000, 1001),
    ("30", 30, 1),
    ("50", 50, 1),
    ("59.94", 60000, 1001),
    ("60", 60, 1),
];

/// The script text (CRLF line endings, ASCII apart from the path).
pub fn script(o: &ScriptOptions) -> Result<String, String> {
    let path = o.signpost.to_string_lossy().into_owned();
    if path.trim().is_empty() {
        return Err("Choose the signpost .avi file first.".into());
    }
    let mut args = vec![quote(&path)?];
    args.push(format!("audio={}", if o.audio == AudioMode::None { "false" } else { "true" }));
    if let Some(pt) = o.pixel_type.value() {
        args.push(format!("pixel_type=\"{pt}\""));
    }
    if !path.is_ascii() {
        args.push("utf8=true".into());
    }
    let mut lines = vec![
        format!("# AviSynth script made by {} {} for frameserving.", crate::APP_NAME, crate::APP_VERSION),
        "# The signpost file only works while your editor is serving frames (DebugMode FrameServer running).".to_string(),
        format!("AviSource({})", args.join(", ")),
    ];
    if let Some((a, b)) = o.trim {
        if b != 0 && b < a {
            return Err(format!("Trim: the last frame ({b}) is before the first frame ({a})."));
        }
        if a != 0 || b != 0 {
            lines.push(format!("Trim({a}, {b})"));
        }
    }
    // after Trim, so the trim frame numbers stay the editor's (source) frame numbers
    if let Some((n, d)) = o.fps {
        if n == 0 || d == 0 {
            return Err("The frame rate must be above 0.".into());
        }
        lines.push(format!("ChangeFPS({n}, {d})"));
    }
    if let Some(f) = o.color.function() {
        let mut ca = vec![];
        if let Some(m) = o.matrix.value() {
            ca.push(format!("matrix=\"{m}\""));
        }
        if o.interlaced {
            ca.push("interlaced=true".into());
        }
        lines.push(format!("{f}({})", ca.join(", ")));
    }
    if o.bits == BitDepth::Ten {
        lines.push("ConvertBits(10)".into());
    }
    if o.audio == AudioMode::To16bit {
        lines.push("ConvertAudioTo16bit()".into());
    }
    Ok(lines.join("\r\n") + "\r\n")
}


/// Unusual but still runnable script choices. Empty when the options are the safe defaults.
pub fn script_warnings(o: &ScriptOptions) -> Vec<String> {
    super::conflicts::messages(&super::conflicts::script_conflicts(o))
}

/// Unusual encode choices (range, tags, bit depth). These are warnings, not errors — the command is still built as requested.
pub fn encode_choice_warnings(pr: &EncodeParams) -> Vec<String> {
    super::conflicts::messages(&super::conflicts::encode_conflicts(pr))
}

/// Unique temporary script path in %TEMP%.
pub fn temp_script_path() -> PathBuf {
    std::env::temp_dir().join(format!("{TEMP_PREFIX}{}.avs", crate::util::unique_id()))
}

/// Script saved next to the signpost (or in `dir`): `<signpost name>_ffenc.avs`.
pub fn saved_script_path(signpost: &Path, dir: Option<&Path>) -> PathBuf {
    let dir = dir.map(|d| d.to_path_buf()).or_else(|| signpost.parent().map(|p| p.to_path_buf())).unwrap_or_default();
    dir.join(format!("{}_ffenc.avs", crate::util::stem(signpost)))
}

/// Encode settings (same choices as Re-encode).
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeParams {
    pub script_path: PathBuf,
    pub script_text: String,
    pub output: PathBuf,
    pub video: VideoEncode,
    pub audio: AudioEncode,
    pub has_audio: bool,
    pub pix_fmt: String,
    pub resize: Option<(u32, u32)>,
    /// Tag the output BT.709 (matrix Rec709 conversion to YUV). Kept so the historic
    /// Rec709 command (tv + bt709 tags, no range filter) stays byte-identical.
    pub bt709_tags: bool,
    /// Colour matrix used in the AviSynth script. Rec709/Rec601/Rec2020 already squeeze
    /// to TV range in the script — a second full→TV filter is never applied.
    pub matrix: Matrix,
    /// When the script uses PC.709 / PC.601 (full range), optionally squeeze to TV range
    /// in FFmpeg. Ignored for matrices that already squeeze in the script.
    pub range_convert: bool,
    pub chapters: Option<(PathBuf, String)>,
    pub faststart: bool,
    /// Crop / watermark (default = none, the historic command).
    pub picture: super::picture::PictureOps,
}

/// `-y -i script.avs [-f ffmetadata -i CH] -map 0:v:0 [-map 0:a:0?] <x264> [<aac>] [-map_chapters 1] [faststart] OUT`
pub fn build(pr: &EncodeParams) -> Result<BuiltCommand, String> {
    if pr.script_text.trim().is_empty() {
        return Err("The AviSynth script is empty.".into());
    }
    if !pr.script_text.contains("Source(") && !pr.script_text.to_ascii_lowercase().contains("source(") {
        return Err("The script has no source line (e.g. AviSource(\"…\")).".into());
    }
    if pr.has_audio {
        pr.audio.validate()?;
    }
    let mp4 = super::reencode::is_mp4_like(&pr.output);
    let mut warnings = vec![];
    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(&pr.script_path)];
    let mut side_files = vec![(pr.script_path.clone(), pr.script_text.clone())];
    if let Some((path, text)) = &pr.chapters {
        a.push_all(["-f", "ffmetadata", "-i"]);
        a.push(p(path));
        side_files.push((path.clone(), text.clone()));
    }
    let overlay = pr.picture.overlay.as_ref();
    let wm_input = 1 + pr.chapters.is_some() as usize;
    if let Some(ov) = overlay {
        a.extend(super::picture::image_input_args(ov));
        a.push_all(["-map", "[vout]"]);
    } else {
        a.push_all(["-map", "0:v:0"]);
    }
    if pr.has_audio {
        a.push_all(["-map", "0:a:0?"]);
    }
    let resize = pr.resize.map(|(w, h)| format!("scale={w}:{h}"));
    // Honour range_convert even when the script already squeezed to TV range. That combo is
    // unusual (washed-out picture) and is warned, not blocked or silently dropped.
    warnings.extend(encode_choice_warnings(pr));
    let squeezed = pr.range_convert;
    let squeeze = squeezed.then(|| super::codecs::range_filter_pix(&pr.video, &pr.pix_fmt));
    let crop = pr.picture.crop_filter();
    // Historic Rec709 path: bt709_tags + video_args colour tags (tv / bt709), no extra filter.
    // PC matrices get their own tags (pc, or tv after a squeeze). Rec601 / Rec2020 are tagged
    // to match the script; they never take the historic bt709_tags shortcut.
    // Rec601 / Rec2020 always use their own tags. Historic Rec709 uses bt709_tags (tv + bt709).
    // If the user asks for those TV tags on an unsqueezed PC matrix, honour it and warn
    // (do not silently retag as full range).
    let classic_bt709 = pr.bt709_tags && !matches!(pr.matrix, Matrix::Rec601 | Matrix::Rec2020);
    let extra_tags = if classic_bt709 {
        vec![]
    } else if pr.matrix.keeps_full_range() || pr.matrix == Matrix::Rec601 || pr.matrix == Matrix::Rec2020 {
        pr.matrix.color_tag_args(squeezed)
    } else {
        vec![]
    };
    // Colour format + script bit depth decide -pix_fmt (may be 4:2:2 / 4:4:4, not only 4:2:0).
    // That override is applied after video_args so a 10-bit codec is not later forced back to 8-bit,
    // and an 8-bit script is not later forced up to 10-bit.
    warnings.extend(super::codecs::check_video(&pr.video, &pr.output)?);
    let mut x = if let Some(ov) = overlay {
        let post = with_output_fps(join_filters(&[resize, squeeze]).as_deref(), &pr.video);
        a.push("-filter_complex".into());
        a.push(super::picture::overlay_graph("0:v:0", crop.as_deref(), ov, wm_input, post.as_deref(), "vout"));
        super::codecs::video_args(&VideoEncode { output_fps: String::new(), ..pr.video.clone() }, None, classic_bt709, &pr.output)
    } else {
        let filter = join_filters(&[crop, resize, squeeze]);
        super::codecs::video_args(&pr.video, filter.as_deref(), classic_bt709, &pr.output)
    };
    if !extra_tags.is_empty() {
        x.extend(extra_tags);
    }
    if let Some(i) = x.iter().position(|s| s == "-pix_fmt") {
        x[i + 1] = pr.pix_fmt.clone();
    } else {
        x.push("-pix_fmt".into());
        x.push(pr.pix_fmt.clone());
    }
    a.extend(x);
    if pr.pix_fmt != "yuv420p" {
        warnings.push(format!("Pixel format {}: fine for archiving, but many TVs, phones and browsers only play 8-bit 4:2:0 (yuv420p).", pr.pix_fmt));
    }
    if pr.has_audio {
        warnings.extend(super::codecs::check_audio(&pr.audio, &pr.output)?);
        a.extend(super::codecs::audio_args(&pr.audio));
        if let Some(chain) = pr.audio.filter_chain().filter(|_| pr.audio.codec != super::codecs::AudioCodec::Copy) {
            a.push_all(["-filter:a".to_string(), chain]);
        }
    } else {
        a.push("-an".into());
    }
    if pr.chapters.is_some() {
        a.push_all(["-map_chapters", "1"]);
    }
    if pr.faststart && mp4 {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, side_files, warnings })
}

/// Turn an ffprobe/ffmpeg error about a script into plain advice.
pub fn explain_error(stderr: &str) -> String {
    let s = stderr.to_ascii_lowercase();
    let tip = if s.contains("unknown input format") || s.contains("avisynth") && s.contains("not found") && s.contains("format") {
        "This FFmpeg can't read AviSynth scripts (no avisynth demuxer). Use a standard Windows build (BtbN or gyan.dev, Help › Check for FFmpeg updates)."
    } else if s.contains("decompressor") || s.contains("couldn't open") || s.contains("could not open") || s.contains("unable to open") || s.contains("avisource") {
        "The signpost couldn't be opened. Make sure the editor (Premiere, Vegas, After Effects…) is open with the project and DebugMode FrameServer is serving – the signpost .avi only works while it is."
    } else if s.contains("unknown error") || s.contains("error number -") || s.trim().is_empty() {
        "AviSynth didn't load. Install AviSynth+ (64-bit, matching this 64-bit FFmpeg) and make sure the frameserver is serving."
    } else if s.contains("there is no function named") {
        "The script uses a function this AviSynth doesn't have. ConvertToYUV420/YUV422/YUV444 and ConvertBits need AviSynth+ (not the old AviSynth 2.6) – install AviSynth+ or pick YV12 / None."
    } else if s.contains("script error") {
        "The script has a mistake – check the line mentioned above, or reset it to the generated script."
    } else {
        "Check that the editor is serving frames and that AviSynth+ (64-bit) is installed."
    };
    let first = stderr.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).take(4).collect::<Vec<_>>().join("\n");
    if first.is_empty() {
        tip.to_string()
    } else {
        format!("{tip}\n\nDetails: {first}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<&str> {
        s.split("\r\n").filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
    }

    #[test]
    fn default_script_is_yuv420_pc709() {
        let o = ScriptOptions::new(r"C:\Users\you\Documents\clip.avi");
        assert_eq!(o.color, Color::Yuv420);
        assert_eq!(o.matrix, Matrix::Pc709);
        assert_eq!(Color::default(), Color::Yuv420);
        let s = script(&o).unwrap();
        assert!(s.ends_with("\r\n") && !s.starts_with('\u{feff}'));
        assert!(!s.contains('\n') || s.matches('\n').count() == s.matches("\r\n").count(), "CRLF only");
        assert_eq!(lines(&s), [r#"AviSource("C:\Users\you\Documents\clip.avi", audio=true)"#, r#"ConvertToYUV420(matrix="PC.709")"#]);
    }

    #[test]
    fn quoting_spaces_unicode_quotes() {
        assert_eq!(quote(r"C:\My Videos\a b.avi").unwrap(), r#""C:\My Videos\a b.avi""#);
        assert_eq!(quote(r"D:\it's\x.avi").unwrap(), r#""D:\it's\x.avi""#);
        assert_eq!(quote(r#"\\srv\say "hi"\x.avi"#).unwrap(), r#""""\\srv\say "hi"\x.avi""""#);
        assert!(quote("a\"\"\"b").is_err());
        assert!(quote("ends with\"").is_err());
        assert!(quote("two\nlines").is_err());
        // unicode path: kept verbatim (UTF-8) with utf8=true
        let s = script(&ScriptOptions::new(r"C:\Users\Jürgen\視頻 1.avi")).unwrap();
        assert!(lines(&s)[0] == r#"AviSource("C:\Users\Jürgen\視頻 1.avi", audio=true, utf8=true)"#, "{s}");
        // header comments are ASCII only
        assert!(s.lines().filter(|l| l.starts_with('#')).all(|l| l.is_ascii()));
        assert!(script(&ScriptOptions::new("")).is_err());
    }

    #[test]
    fn option_combos() {
        let mut o = ScriptOptions::new(r"C:\a.avi");
        o.pixel_type = PixelType::Yuy2;
        o.audio = AudioMode::To16bit;
        o.color = Color::Yuv422;
        o.matrix = Matrix::Pc709;
        o.interlaced = true;
        o.trim = Some((100, 2000));
        o.fps = Some((30000, 1001));
        o.bits = BitDepth::Ten;
        assert_eq!(
            lines(&script(&o).unwrap()),
            [
                r#"AviSource("C:\a.avi", audio=true, pixel_type="YUY2")"#,
                "Trim(100, 2000)",
                "ChangeFPS(30000, 1001)",
                r#"ConvertToYUV422(matrix="PC.709", interlaced=true)"#,
                "ConvertBits(10)",
                "ConvertAudioTo16bit()",
            ]
        );
        // no audio, no conversion, default matrix, trim to the end
        let mut o = ScriptOptions::new(r"C:\a.avi");
        o.audio = AudioMode::None;
        o.color = Color::None;
        o.trim = Some((50, 0));
        assert_eq!(lines(&script(&o).unwrap()), [r#"AviSource("C:\a.avi", audio=false)"#, "Trim(50, 0)"]);
        // Trim(0, 0) = whole clip: no line
        o.trim = Some((0, 0));
        assert_eq!(lines(&script(&o).unwrap()).len(), 1);
        // every colour option maps to the documented AviSynth+ function
        let names: Vec<_> = Color::ALL.iter().map(|c| c.function()).collect();
        assert_eq!(names, [Some("ConvertToYUV420"), Some("ConvertToYV12"), Some("ConvertToYUV422"), Some("ConvertToYUV444"), Some("ConvertToRGB32"), None]);
        for c in [Color::Yv12, Color::Rgb32] {
            let mut o = ScriptOptions::new(r"C:\a.avi");
            o.color = c;
            o.matrix = Matrix::Default;
            assert_eq!(lines(&script(&o).unwrap())[1], format!("{}()", c.function().unwrap()));
            o.bits = BitDepth::Ten;
            let s = script(&o).unwrap();
            assert!(s.contains("ConvertBits(10)"), "{c:?} still writes ConvertBits: {s}");
            assert!(script_warnings(&o).iter().any(|w| w.contains("8-bit") && w.contains("still run")), "{c:?} {:?}", script_warnings(&o));
        }
        // bad trims / fps
        let mut o = ScriptOptions::new(r"C:\a.avi");
        o.trim = Some((10, 5));
        assert!(script(&o).is_err());
        o.trim = None;
        o.fps = Some((0, 1));
        assert!(script(&o).is_err());
        // pixel formats
        assert_eq!(Color::Yuv420.pix_fmt(BitDepth::Eight), "yuv420p");
        assert_eq!(Color::Yuv444.pix_fmt(BitDepth::Ten), "yuv444p10le");
        assert_eq!(Color::Rgb32.pix_fmt(BitDepth::Eight), "yuv420p");
        assert!(Color::ALL.iter().all(|c| !c.help().is_empty() && c.label().contains(c.function().unwrap_or("None"))));
    }

    /// A changed frame rate is a real conversion (ChangeFPS, after Trim), never AssumeFPS, which
    /// changes the speed of the video only and puts the audio out of sync (1.1 bug report).
    #[test]
    fn frame_rate_change_keeps_audio_in_sync() {
        let mut o = ScriptOptions::new(r"C:\sp.avi");
        let plain = script(&o).unwrap();
        for (n, d) in [(24000, 1001), (30, 1), (60, 1)] {
            o.fps = Some((n, d));
            o.trim = Some((10, 0));
            let s = script(&o).unwrap();
            assert!(!s.contains("AssumeFPS"), "{s}");
            let l = lines(&s);
            let t = l.iter().position(|x| x.starts_with("Trim(")).unwrap();
            assert_eq!(l[t + 1], format!("ChangeFPS({n}, {d})"), "after Trim: trim frames are source frames");
        }
        // left at the default: the script is exactly as before
        o.fps = None;
        o.trim = None;
        assert_eq!(script(&o).unwrap(), plain);
        assert!(!plain.contains("FPS("));
    }

    fn enc() -> EncodeParams {
        let o = ScriptOptions::new(r"C:\a.avi");
        EncodeParams {
            script_path: PathBuf::from(r"C:\T\ffenc_frameserve_1.avs"),
            script_text: script(&o).unwrap(),
            output: PathBuf::from(r"C:\out\a_frameserved.mp4"),
            video: VideoEncode::default(),
            audio: AudioEncode::default(),
            has_audio: true,
            pix_fmt: "yuv420p".into(),
            resize: None,
            bt709_tags: false,
            matrix: Matrix::Rec709,
            range_convert: false,
            chapters: None,
            faststart: true,
            picture: Default::default(),
        }
    }

    #[test]
    fn encode_command() {
        let b = build(&enc()).unwrap();
        assert_eq!(
            b.args,
            [
                "-y", "-i", r"C:\T\ffenc_frameserve_1.avs", "-map", "0:v:0", "-map", "0:a:0?",
                "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", r"C:\out\a_frameserved.mp4"
            ]
        );
        assert_eq!(b.side_files.len(), 1);
        assert_eq!(b.side_files[0].0, PathBuf::from(r"C:\T\ffenc_frameserve_1.avs"));
        assert!(b.side_files[0].1.contains("AviSource("));
        assert!(b.warnings.is_empty());
        assert_eq!(b.args.last().unwrap(), r"C:\out\a_frameserved.mp4", "output last (overwrite guard rewrites it)");
        // no audio, 10-bit 4:2:2, resize, tags, chapters, mkv (no faststart)
        let mut pr = enc();
        pr.has_audio = false;
        pr.pix_fmt = Color::Yuv422.pix_fmt(BitDepth::Ten);
        pr.resize = Some((1280, 720));
        pr.bt709_tags = true;
        pr.video.tune = Some("film".into());
        pr.audio.gain_db = 3.0;
        pr.chapters = Some((PathBuf::from(r"C:\T\ch.ffmeta"), ";FFMETADATA1\n".into()));
        pr.output = PathBuf::from(r"C:\out\a.mkv");
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.starts_with(r"-y -i C:\T\ffenc_frameserve_1.avs -f ffmetadata -i C:\T\ch.ffmeta -map 0:v:0 -c:v libx264"), "{s}");
        assert!(s.contains("-pix_fmt yuv422p10le -vf scale=1280:720 -color_range tv -colorspace bt709"));
        assert!(s.contains("-tune film -an -map_chapters 1"));
        assert!(!s.contains("aac") && !s.contains("faststart") && !s.contains("volume"));
        assert_eq!(b.side_files.len(), 2);
        assert_eq!(b.warnings.len(), 1);
        // gain only with audio
        let mut pr = enc();
        pr.audio.gain_db = -2.0;
        assert!(build(&pr).unwrap().args.windows(2).any(|w| w == ["-filter:a", "volume=-2dB"]));
        // a hand-edited script must still have a source
        let mut pr = enc();
        pr.script_text = "Version()\r\n".into();
        assert!(build(&pr).is_err());
        pr.script_text = "  ".into();
        assert!(build(&pr).is_err());
        pr.script_text = "FFVideoSource(\"x.mkv\")".into();
        assert!(build(&pr).is_ok(), "any *Source( line is accepted");
    }

    #[test]
    fn hevc_and_av1_from_avs_keep_pix_fmt() {
        use super::super::codecs::VideoCodec;
        let mut pr = enc();
        pr.video.codec = VideoCodec::Hevc;
        pr.video.crf = 20;
        pr.video.ten_bit = false;
        pr.pix_fmt = "yuv420p".into();
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-c:v libx265 -crf 20 -preset slow -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("-x265-params log-level=error -tag:v hvc1"), "{s}");
        assert!(s.contains("-c:a aac -b:a 192k"), "{s}");
        assert!(!s.contains("-c:v libx264"), "{s}");
        // 10-bit HEVC: script pix_fmt wins and must not be forced back to 8-bit
        pr.video.ten_bit = true;
        pr.pix_fmt = Color::Yuv420.pix_fmt(BitDepth::Ten);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-pix_fmt yuv420p10le"), "{s}");
        assert!(!b.args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p"]), "{s}");
        assert!(b.warnings.iter().any(|w| w.contains("yuv420p10le") || w.contains("Chrome")), "{:?}", b.warnings);

        let mut pr = enc();
        pr.video.codec = VideoCodec::Av1;
        pr.video.crf = 28;
        pr.video.svt_preset = 5;
        pr.video.ten_bit = false;
        pr.pix_fmt = "yuv420p".into();
        pr.output = PathBuf::from(r"C:\out\a_frameserved.mkv");
        pr.faststart = false;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-c:v libsvtav1 -crf 28 -preset 5 -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("-c:a aac -b:a 192k"), "{s}");
        assert!(!s.contains("-tag:v"), "{s}");
        pr.video.ten_bit = true;
        pr.pix_fmt = Color::Yuv422.pix_fmt(BitDepth::Ten);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-pix_fmt yuv422p10le"), "{s}");
        assert!(!b.args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p"] || w == ["-pix_fmt", "yuv420p10le"]), "{s}");
    }

    #[test]
    fn no_double_range_squeeze_and_pc_tags() {
        use super::super::codecs::VideoCodec;
        // Rec709 already squeezes in the script: default is no second squeeze
        let mut pr = enc();
        pr.bt709_tags = true;
        pr.matrix = Matrix::Rec709;
        pr.range_convert = false;
        pr.pix_fmt = "yuv420p".into();
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-color_range tv -colorspace bt709"), "{s}");
        assert!(!s.contains("in_range=full"), "{s}");
        assert!(encode_choice_warnings(&pr).is_empty());

        // User override: Rec709 + range_convert still builds (double squeeze) and warns, does not block
        pr.range_convert = true;
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("scale=in_range=full:out_range=tv"), "{s}");
        assert!(s.contains("-color_range tv -colorspace bt709"), "{s}");
        assert!(b.warnings.iter().any(|w| w.contains("washed out")), "{:?}", b.warnings);

        // Rec601 / Rec2020: tag TV range; no second squeeze unless the user asked
        pr.range_convert = false;
        pr.bt709_tags = false;
        pr.matrix = Matrix::Rec601;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-color_range tv -colorspace smpte170m"), "{s}");
        assert!(!s.contains("in_range=full"), "{s}");
        pr.matrix = Matrix::Rec2020;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-color_range tv -colorspace bt2020nc"), "{s}");
        assert!(!s.contains("in_range=full"), "{s}");

        // PC.709 without convert: tag full range, never as tv
        pr.matrix = Matrix::Pc709;
        pr.range_convert = false;
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-color_range pc -colorspace bt709"), "{s}");
        assert!(!s.contains("-color_range tv"), "{s}");
        assert!(!s.contains("in_range=full"), "{s}");

        // PC.709 + convert: one squeeze, then TV tags (not pc)
        pr.range_convert = true;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("scale=in_range=full:out_range=tv"), "{s}");
        assert!(s.contains("-color_range tv -colorspace bt709"), "{s}");
        assert!(!s.contains("-color_range pc"), "{s}");

        // 10-bit HEVC + PC.709 convert: squeeze and 10-bit format in one step; -pix_fmt stays 10-bit
        pr.video.codec = VideoCodec::Hevc;
        pr.video.crf = 20;
        pr.video.ten_bit = true;
        pr.pix_fmt = Color::Yuv420.pix_fmt(BitDepth::Ten);
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("scale=in_range=full:out_range=tv,format=yuv420p10le"), "{s}");
        assert!(b.args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p10le"]), "{s}");
        assert!(!b.args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p"]), "{s}");

        // 10-bit Rec709 (ConvertBits in the script), no extra squeeze: pix_fmt stays 10-bit
        pr.matrix = Matrix::Rec709;
        pr.range_convert = false;
        pr.bt709_tags = true;
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(!s.contains("in_range=full"), "{s}");
        assert!(b.args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p10le"]), "{s}");
        assert!(s.contains("-color_range tv -colorspace bt709"), "{s}");

        // HEVC 10-bit ticked, 8-bit script: warn, still 8-bit pix_fmt, encode runs
        pr.video.ten_bit = true;
        pr.pix_fmt = "yuv420p".into();
        let b = build(&pr).unwrap();
        assert!(b.args.windows(2).any(|w| w == ["-pix_fmt", "yuv420p"]));
        assert!(b.warnings.iter().any(|w| w.contains("script bit depth is 8-bit")), "{:?}", b.warnings);

        // PC.709 + TV tags without a squeeze: warn about wrong tags, encode still runs, tags are as requested
        let mut pr = enc();
        pr.matrix = Matrix::Pc709;
        pr.range_convert = false;
        pr.bt709_tags = true;
        let b = build(&pr).unwrap();
        assert!(b.warnings.iter().any(|w| w.contains("crush the blacks") || w.contains("full-range picture")), "{:?}", b.warnings);
        // Honour the requested TV tags (the unusual combo) rather than silently retagging as pc
        let s = b.args.join(" ");
        assert!(s.contains("-color_range tv"), "{s}");
        assert!(!s.contains("-color_range pc"), "{s}");
    }

    #[test]
    fn paths_and_errors() {
        let t = temp_script_path();
        let n = t.file_name().unwrap().to_string_lossy().into_owned();
        assert!(n.starts_with(TEMP_PREFIX) && n.ends_with(".avs"));
        assert_ne!(temp_script_path(), t);
        assert_eq!(saved_script_path(Path::new("/v/my clip.avi"), None), PathBuf::from("/v/my clip_ffenc.avs"));
        assert_eq!(saved_script_path(Path::new("/v/x.avi"), Some(Path::new("/s"))), PathBuf::from("/s/x_ffenc.avs"));
        assert!(explain_error("AVISource: couldn't locate a decompressor for fourcc dfsc").contains("serving"));
        assert!(explain_error("x.avs: Unknown error occurred").contains("AviSynth+"));
        assert!(explain_error("Unknown input format: 'avisynth'").contains("demuxer"));
        assert!(explain_error("Script error: There is no function named 'ConvertToYUV420'.").contains("AviSynth+"));
    }

    #[test]
    fn crop_and_watermark_in_frameserve() {
        use crate::cmd::picture::{CropPx, OverlayPx};
        let mut pr = enc();
        let off = build(&pr).unwrap();
        pr.picture.crop = Some(CropPx { w: 1280, h: 536, x: 0, y: 92 });
        let j = build(&pr).unwrap().args.join(" ");
        assert!(j.contains("-vf crop=1280:536:0:92"), "{j}");
        pr.picture.overlay = Some(OverlayPx { image: "/w.png".into(), w: 100, h: None, x: 5, y: 6, opacity: 0.25 });
        let b = build(&pr).unwrap();
        let j = b.args.join(" ");
        assert!(j.contains("-i /w.png -map [vout]"), "{j}");
        assert!(j.contains("-filter_complex [2:v]scale=100:-1,format=rgba,colorchannelmixer=aa=0.25[wm];[0:v:0]crop=1280:536:0:92[base];[base][wm]overlay=x=5:y=6:format=auto:eof_action=repeat[vout]") || j.contains("-filter_complex [1:v]scale=100:-1"), "{j}");
        assert!(!j.contains("-vf"), "{j}");
        pr.picture = Default::default();
        assert_eq!(build(&pr).unwrap(), off);
    }
}
