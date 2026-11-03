//! Frameserving (DebugMode FrameServer and similar): an editor serves frames
//! through a small "signpost" .avi; an AviSynth script opens it with AviSource
//! and ffmpeg reads the script (`-i script.avs`, avisynth demuxer).
//!
//! Function names follow the AviSynth+ documentation (avisynth.nl "Convert",
//! "AviSource", "ConvertBits", "AssumeFPS", "Trim"). Strings in AviSynth have no
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
    pub const ALL: [Matrix; 6] = [Matrix::Rec709, Matrix::Rec601, Matrix::Pc709, Matrix::Pc601, Matrix::Rec2020, Matrix::Default];
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
            Matrix::Rec709 => "Rec709 – HD and most editor projects (recommended)",
            Matrix::Rec601 => "Rec601 – standard definition (DVD, 480/576 lines)",
            Matrix::Pc709 => "PC.709 – HD, keep full range",
            Matrix::Pc601 => "PC.601 – SD, keep full range",
            Matrix::Rec2020 => "Rec2020 – UHD / HDR projects",
        }
    }
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
    /// (numerator, denominator).
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
            matrix: Matrix::Rec709,
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
    if o.bits == BitDepth::Ten && matches!(o.color, Color::Yv12 | Color::Rgb32) {
        return Err(format!("10-bit can't be combined with {} (it is 8-bit only). Pick YUV420, YUV422, YUV444 or None.", o.color.function().unwrap_or("")));
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
        format!("# AviSynth script made by {} for frameserving.", crate::full_title()),
        "# The signpost file only works while your editor is serving frames (DebugMode FrameServer running).".to_string(),
        format!("AviSource({})", args.join(", ")),
    ];
    if let Some((n, d)) = o.fps {
        if n == 0 || d == 0 {
            return Err("The frame rate must be above 0.".into());
        }
        lines.push(format!("AssumeFPS({n}, {d})"));
    }
    if let Some((a, b)) = o.trim {
        if b != 0 && b < a {
            return Err(format!("Trim: the last frame ({b}) is before the first frame ({a})."));
        }
        if a != 0 || b != 0 {
            lines.push(format!("Trim({a}, {b})"));
        }
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
    /// Tag the output BT.709 (matrix Rec709 conversion to YUV).
    pub bt709_tags: bool,
    pub chapters: Option<(PathBuf, String)>,
    pub faststart: bool,
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
    a.push_all(["-map", "0:v:0"]);
    if pr.has_audio {
        a.push_all(["-map", "0:a:0?"]);
    }
    let filter = pr.resize.map(|(w, h)| format!("scale={w}:{h}"));
    let mut x = x264_args(&pr.video, filter.as_deref(), pr.bt709_tags);
    if let Some(i) = x.iter().position(|s| s == "-pix_fmt") {
        x[i + 1] = pr.pix_fmt.clone();
    }
    a.extend(x);
    if pr.pix_fmt != "yuv420p" {
        warnings.push(format!("Pixel format {}: fine for archiving, but many TVs, phones and browsers only play 8-bit 4:2:0 (yuv420p).", pr.pix_fmt));
    }
    if pr.has_audio {
        a.extend(pr.audio.aac_args());
        if let Some(chain) = pr.audio.filter_chain() {
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
    fn default_script_is_yuv420_rec709() {
        let o = ScriptOptions::new(r"C:\Users\alex\Documents\clip.avi");
        assert_eq!(o.color, Color::Yuv420);
        assert_eq!(Color::default(), Color::Yuv420);
        let s = script(&o).unwrap();
        assert!(s.ends_with("\r\n") && !s.starts_with('\u{feff}'));
        assert!(!s.contains('\n') || s.matches('\n').count() == s.matches("\r\n").count(), "CRLF only");
        assert_eq!(lines(&s), [r#"AviSource("C:\Users\alex\Documents\clip.avi", audio=true)"#, r#"ConvertToYUV420(matrix="Rec709")"#]);
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
                "AssumeFPS(30000, 1001)",
                "Trim(100, 2000)",
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
            assert!(script(&o).is_err(), "{c:?} is 8-bit only");
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
            chapters: None,
            faststart: true,
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
}
