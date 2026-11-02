//! "Audio & subtitles": one pass over a video that keeps, removes, reorders and renames its
//! audio and subtitle tracks, adds tracks from other files (commentary, second language,
//! subtitle files), replaces audio, and – as separate jobs – extracts audio or subtitle tracks.
//!
//! The video is always stream-copied. Audio is copied when the container accepts the codec
//! and nothing has to be changed, otherwise encoded (AAC; FLAC for lossless sources in MKV).
//! Subtitles: MP4 only takes text subtitles (converted to mov_text); MKV takes everything.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::common::{CompressorParams, CompressorPreset};
use super::{p, ArgVec, BuiltCommand};
use crate::util::fmt_num;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Container {
    #[default]
    Mkv,
    Mp4,
}

impl Container {
    pub fn of(path: &Path) -> Option<Container> {
        match crate::util::ext_lower(path).as_str() {
            "mkv" => Some(Container::Mkv),
            "mp4" | "m4v" | "mov" => Some(Container::Mp4),
            _ => None,
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mkv => "mkv",
            Container::Mp4 => "mp4",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Container::Mkv => "MKV",
            Container::Mp4 => "MP4",
        }
    }
    /// Plain-language difference, shown next to the choice.
    pub fn explain(self) -> &'static str {
        match self {
            Container::Mkv => "MKV keeps every kind of audio and subtitle track as it is (including picture subtitles and styled ASS subtitles). Most players and editors open it.",
            Container::Mp4 => "MP4 plays almost everywhere (phones, TVs, browsers) but is pickier: audio that MP4 can't hold is converted to AAC, text subtitles are converted to the simple MP4 subtitle format (styling is lost), and picture subtitles can't be included.",
        }
    }
}

/// Where a track comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum Source {
    /// The video being edited (input 0).
    #[default]
    Main,
    /// Another file (audio file, subtitle file or another video).
    File(PathBuf),
}

/// What to do when an added track is longer or shorter than the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Fit {
    /// Longer: trim to the video length. Shorter: leave as is.
    #[default]
    Auto,
    /// Keep the whole track (the file may run longer than the picture).
    KeepFull,
    /// Cut the track at the end of the video.
    Trim,
    /// Leave a shorter track as it is (it just ends early).
    Leave,
    /// Fill a shorter audio track with silence up to the video length.
    Pad,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioTrack {
    pub source: Source,
    /// Index among the audio streams of its source (`N:a:<stream>`).
    pub stream: usize,
    pub codec: String,
    pub channels: Option<u32>,
    /// Length of the track/file in seconds (external tracks; for the length check).
    pub duration: Option<f64>,
    pub keep: bool,
    pub title: String,
    pub language: String,
    pub default: bool,
    /// Positive: starts later; negative: the beginning of the file is skipped.
    pub delay_ms: i64,
    pub gain_db: f64,
    pub compressor: CompressorPreset,
    pub fit: Fit,
    /// Also add a track that mixes this one into the main (first kept original) audio.
    pub mix_with_main: bool,
}

impl Default for AudioTrack {
    fn default() -> Self {
        AudioTrack {
            source: Source::Main,
            stream: 0,
            codec: String::new(),
            channels: None,
            duration: None,
            keep: true,
            title: String::new(),
            language: String::new(),
            default: false,
            delay_ms: 0,
            gain_db: 0.0,
            compressor: CompressorPreset::None,
            fit: Fit::Auto,
            mix_with_main: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubTrack {
    pub source: Source,
    /// Index among the subtitle streams of its source (`N:s:<stream>`).
    pub stream: usize,
    pub codec: String,
    pub duration: Option<f64>,
    pub keep: bool,
    pub title: String,
    pub language: String,
    pub default: bool,
    pub forced: bool,
    pub delay_ms: i64,
    /// Text encoding of an external text subtitle file ("" = UTF-8 / automatic).
    pub charset: String,
}

impl Default for SubTrack {
    fn default() -> Self {
        SubTrack {
            source: Source::Main,
            stream: 0,
            codec: String::new(),
            duration: None,
            keep: true,
            title: String::new(),
            language: String::new(),
            default: false,
            forced: false,
            delay_ms: 0,
            charset: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TracksParams {
    pub video: PathBuf,
    /// Video length in seconds (for trimming / padding / the length check).
    pub video_duration: Option<f64>,
    pub audio: Vec<AudioTrack>,
    pub subs: Vec<SubTrack>,
    pub output: PathBuf,
    pub aac_bitrate_k: u32,
    /// Values for `CompressorPreset::Advanced`.
    pub advanced: CompressorParams,
    /// Differences up to this many seconds are "the same length".
    pub tolerance_s: f64,
    pub faststart: bool,
}

// ---------------------------------------------------------------- codecs

const MP4_AUDIO: &[&str] = &["aac", "mp3", "ac3", "eac3", "alac"];
const LOSSLESS: &[&str] = &["flac", "alac", "truehd", "mlp", "wavpack", "tta"];
const TEXT_SUBS: &[&str] = &[
    "subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text", "microdvd", "subviewer", "subviewer1", "sami", "realtext", "jacosub",
    "mpl2", "pjs", "stl", "vplayer", "eia_608",
];

pub fn is_lossless(codec: &str) -> bool {
    codec.starts_with("pcm_") || LOSSLESS.contains(&codec)
}

/// Can the container hold this audio codec as it is?
pub fn audio_fits(c: Container, codec: &str) -> bool {
    match c {
        Container::Mp4 => MP4_AUDIO.contains(&codec),
        Container::Mkv => !codec.is_empty() && !matches!(codec, "pcm_dvd" | "pcm_bluray"),
    }
}

pub fn is_text_sub(codec: &str) -> bool {
    TEXT_SUBS.contains(&codec)
}

/// Human name of an audio codec.
pub fn audio_codec_name(codec: &str) -> String {
    match codec {
        "aac" => "AAC".into(),
        "mp3" => "MP3".into(),
        "ac3" => "Dolby Digital (AC-3)".into(),
        "eac3" => "Dolby Digital Plus".into(),
        "flac" => "FLAC".into(),
        "opus" => "Opus".into(),
        "vorbis" => "Vorbis".into(),
        "alac" => "Apple Lossless".into(),
        "dts" => "DTS".into(),
        "truehd" => "Dolby TrueHD".into(),
        c if c.starts_with("pcm_") => "PCM (WAV)".into(),
        "" => "unknown".into(),
        c => c.to_string(),
    }
}

pub fn sub_codec_name(codec: &str) -> String {
    match codec {
        "subrip" | "srt" => "SRT text".into(),
        "ass" | "ssa" => "ASS/SSA styled text".into(),
        "webvtt" => "WebVTT text".into(),
        "mov_text" => "MP4 text".into(),
        "hdmv_pgs_subtitle" => "Blu-ray picture (PGS)".into(),
        "dvd_subtitle" => "DVD picture (VobSub)".into(),
        "dvb_subtitle" => "DVB picture".into(),
        "" => "unknown".into(),
        c if is_text_sub(c) => format!("{c} text"),
        c => format!("{c} (picture)"),
    }
}

// ---------------------------------------------------------------- language

/// Common languages: (ISO 639-2 code, 2-letter code, English name).
pub const LANGUAGES: &[(&str, &str, &str)] = &[
    ("eng", "en", "English"), ("spa", "es", "Spanish"), ("fre", "fr", "French"), ("ger", "de", "German"), ("ita", "it", "Italian"),
    ("por", "pt", "Portuguese"), ("dut", "nl", "Dutch"), ("swe", "sv", "Swedish"), ("nor", "no", "Norwegian"), ("dan", "da", "Danish"),
    ("fin", "fi", "Finnish"), ("pol", "pl", "Polish"), ("cze", "cs", "Czech"), ("hun", "hu", "Hungarian"), ("rus", "ru", "Russian"),
    ("ukr", "uk", "Ukrainian"), ("gre", "el", "Greek"), ("tur", "tr", "Turkish"), ("ara", "ar", "Arabic"), ("heb", "he", "Hebrew"),
    ("hin", "hi", "Hindi"), ("jpn", "ja", "Japanese"), ("chi", "zh", "Chinese"), ("kor", "ko", "Korean"), ("tha", "th", "Thai"),
    ("vie", "vi", "Vietnamese"), ("ind", "id", "Indonesian"), ("und", "", "Undetermined"),
];

/// Normalise what the user typed to a 3-letter code: "en" / "English" / "ENG" -> "eng".
/// Empty -> "und". Returns None for something unrecognisable.
pub fn normalize_language(s: &str) -> Option<String> {
    let t = s.trim().to_lowercase();
    if t.is_empty() {
        return Some("und".into());
    }
    for (c3, c2, name) in LANGUAGES {
        if t == *c3 || (!c2.is_empty() && t == *c2) || t == name.to_lowercase() {
            return Some(c3.to_string());
        }
    }
    // ISO 639-2/B vs /T variants players also understand
    if t.len() == 3 && t.chars().all(|c| c.is_ascii_lowercase()) {
        return Some(t);
    }
    None
}

pub fn language_name(code: &str) -> String {
    LANGUAGES.iter().find(|l| l.0 == code || (!l.1.is_empty() && l.1 == code)).map(|l| l.2.to_string()).unwrap_or_else(|| code.to_string())
}

// ---------------------------------------------------------------- charset

/// Text encodings offered for subtitle files: (ffmpeg/iconv name, label).
pub const CHARSETS: &[(&str, &str)] = &[
    ("", "Automatic (UTF-8, or Western if not UTF-8)"),
    ("UTF-8", "UTF-8"),
    ("CP1252", "Western European (Windows-1252)"),
    ("ISO-8859-1", "Western European (ISO-8859-1)"),
    ("ISO-8859-15", "Western European with € (ISO-8859-15)"),
    ("CP1250", "Central European (Windows-1250)"),
    ("ISO-8859-2", "Central European (ISO-8859-2)"),
    ("CP1251", "Cyrillic (Windows-1251)"),
    ("KOI8-R", "Russian (KOI8-R)"),
    ("CP1253", "Greek (Windows-1253)"),
    ("CP1254", "Turkish (Windows-1254)"),
    ("CP1255", "Hebrew (Windows-1255)"),
    ("CP1256", "Arabic (Windows-1256)"),
    ("SHIFT_JIS", "Japanese (Shift-JIS)"),
    ("GBK", "Chinese simplified (GBK)"),
    ("BIG5", "Chinese traditional (Big5)"),
    ("EUC-KR", "Korean (EUC-KR)"),
];

/// Guess the encoding of a subtitle file: None when it is UTF-8 (with or without BOM) or
/// UTF-16 with BOM (ffmpeg reads those itself), otherwise Windows-1252.
pub fn detect_charset(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return None;
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if std::str::from_utf8(b).is_ok() {
        None
    } else {
        Some("CP1252")
    }
}

/// The charset actually passed to ffmpeg for a track ("" = none).
pub fn effective_charset(t: &SubTrack) -> String {
    if !matches!(t.source, Source::File(_)) || !is_text_sub(&t.codec) {
        return String::new();
    }
    if t.charset.is_empty() {
        if let Source::File(f) = &t.source {
            if let Ok(bytes) = std::fs::read(f) {
                return detect_charset(&bytes).unwrap_or("").to_string();
            }
        }
        return String::new();
    }
    if t.charset.eq_ignore_ascii_case("UTF-8") {
        return String::new();
    }
    t.charset.clone()
}

// ---------------------------------------------------------------- lengths

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// Within the tolerance.
    Matches,
    Longer,
    Shorter,
}

/// Compare an added track (with its delay) to the video. Returns (difference in seconds, verdict).
pub fn length_check(video_s: f64, track_s: f64, delay_ms: i64, tolerance_s: f64) -> (f64, Length) {
    let end = track_s + delay_ms as f64 / 1000.0;
    let diff = end - video_s;
    let v = if diff.abs() <= tolerance_s {
        Length::Matches
    } else if diff > 0.0 {
        Length::Longer
    } else {
        Length::Shorter
    };
    (diff, v)
}

/// What will be done: Auto resolves to Trim for any longer track, Leave for a shorter one.
pub fn effective_fit(fit: Fit, diff_s: f64) -> Fit {
    match fit {
        Fit::Auto if diff_s > 0.05 => Fit::Trim,
        Fit::Auto => Fit::Leave,
        Fit::Pad if diff_s >= 0.0 => Fit::Leave,
        f => f,
    }
}

/// Plain-English note about a track's length.
pub fn length_note(video_s: f64, track_s: f64, delay_ms: i64, tolerance_s: f64) -> String {
    let (diff, v) = length_check(video_s, track_s, delay_ms, tolerance_s);
    let d = format!("{:.1}", diff.abs());
    match v {
        Length::Matches if diff.abs() < 0.05 => "Same length as the video.".into(),
        Length::Matches => format!("{d} s {} than the video – close enough (tolerance {} s).", if diff > 0.0 { "longer" } else { "shorter" }, fmt_num(tolerance_s)),
        Length::Longer => format!("{d} s longer than the video. Check that this is the right file and the delay is right."),
        Length::Shorter => format!("{d} s shorter than the video. Check that this is the right file and the delay is right."),
    }
}

// ---------------------------------------------------------------- the one-pass build

fn secs(ms: i64) -> String {
    fmt_num(ms as f64 / 1000.0)
}

/// One ffmpeg input (an external file with its own options).
#[derive(Debug, Clone, PartialEq)]
struct Input {
    path: PathBuf,
    opts: Vec<String>,
}

fn input_index(inputs: &mut Vec<Input>, path: &Path, opts: Vec<String>) -> usize {
    if let Some(i) = inputs.iter().position(|x| x.path == path && x.opts == opts) {
        return i + 1;
    }
    inputs.push(Input { path: path.to_path_buf(), opts });
    inputs.len()
}

fn track_name(title: &str, language: &str, kind: &str, n: usize) -> String {
    if !title.trim().is_empty() {
        title.trim().to_string()
    } else if !language.trim().is_empty() && language != "und" {
        language_name(&normalize_language(language).unwrap_or_default())
    } else {
        format!("{kind} {n}")
    }
}

/// "Output will have: 1 video, 2 audio (Original, Commentary), 3 subtitles (English, …)".
pub fn summary(pr: &TracksParams, video_streams: usize) -> String {
    let c = Container::of(&pr.output).unwrap_or_default();
    let mut audio = vec![];
    let mut n = 0;
    let main_audio = pr.audio.iter().find(|t| t.keep && t.source == Source::Main).map(|t| track_name(&t.title, &t.language, "Audio", t.stream + 1));
    for t in pr.audio.iter().filter(|t| t.keep) {
        n += 1;
        let name = track_name(&t.title, &t.language, "Audio", n);
        if t.mix_with_main && t.source != Source::Main {
            if let Some(m) = &main_audio {
                audio.push(format!("{m} + {name}"));
            }
        }
        audio.insert(audio.len() - usize::from(t.mix_with_main && t.source != Source::Main && main_audio.is_some()), name);
    }
    let mut subs = vec![];
    let mut k = 0;
    for t in pr.subs.iter().filter(|t| t.keep) {
        if c == Container::Mp4 && !is_text_sub(&t.codec) {
            continue;
        }
        k += 1;
        subs.push(track_name(&t.title, &t.language, "Subtitle", k));
    }
    let part = |n: usize, one: &str, many: &str, names: &[String]| {
        let w = if n == 1 { one } else { many };
        if names.is_empty() {
            format!("{n} {w}")
        } else {
            format!("{n} {w} ({})", names.join(", "))
        }
    };
    format!(
        "Output will have: {}, {}, {}",
        part(video_streams, "video", "videos", &[]),
        part(audio.len(), "audio", "audio", &audio),
        part(subs.len(), "subtitle", "subtitles", &subs)
    )
}

/// Build the one-pass command.
pub fn build(pr: &TracksParams) -> Result<BuiltCommand, String> {
    let c = Container::of(&pr.output).ok_or("The output file must end in .mkv or .mp4.")?;
    let mut warnings: Vec<String> = vec![];
    let mut inputs: Vec<Input> = vec![];
    let mut filters: Vec<String> = vec![];
    // (map spec, codec args, title, language, default)
    struct Out {
        map: String,
        codec: Vec<String>,
        title: String,
        lang: String,
        default: bool,
        forced: bool,
    }
    let mut aouts: Vec<Out> = vec![];
    let mut souts: Vec<Out> = vec![];
    let vdur = pr.video_duration;

    let lang_of = |s: &str, what: &str, warnings: &mut Vec<String>| match normalize_language(s) {
        Some(l) => l,
        None => {
            warnings.push(format!("{what}: “{s}” is not a language code ffmpeg knows (use a 3-letter code such as eng, spa, ger); it is stored as undetermined."));
            "und".into()
        }
    };
    let encode_args = |codec: &str, i: usize| -> Vec<String> {
        if c == Container::Mkv && is_lossless(codec) {
            vec![format!("-c:a:{i}"), "flac".into()]
        } else {
            vec![format!("-c:a:{i}"), "aac".into(), format!("-b:a:{i}"), format!("{}k", pr.aac_bitrate_k)]
        }
    };
    let main_ref = pr.audio.iter().find(|t| t.keep && t.source == Source::Main).map(|t| t.stream);

    let mut n_audio = 0usize;
    for (ti, t) in pr.audio.iter().enumerate().filter(|(_, t)| t.keep) {
        n_audio += 1;
        let what = format!("Audio track {n_audio}");
        let lang = lang_of(&t.language, &what, &mut warnings);
        let comp = CompressorParams::for_preset(t.compressor, &pr.advanced);
        if let Some(cp) = comp {
            cp.validate()?;
        }
        if !(-30.0..=30.0).contains(&t.gain_db) {
            return Err(format!("{what}: volume change must be between -30 and +30 dB."));
        }
        let mut chain: Vec<String> = vec![];
        if t.gain_db != 0.0 {
            chain.push(format!("volume={}dB", fmt_num(t.gain_db)));
        }
        if let Some(cp) = comp {
            chain.push(cp.filter());
        }
        let out_i = aouts.len();
        match &t.source {
            Source::Main => {
                if t.delay_ms != 0 {
                    warnings.push(format!("{what}: a delay only applies to added tracks; ignored for the video's own audio."));
                }
                if chain.is_empty() {
                    let codec = if audio_fits(c, &t.codec) { vec![format!("-c:a:{out_i}"), "copy".into()] } else { encode_args(&t.codec, out_i) };
                    aouts.push(Out { map: format!("0:a:{}", t.stream), codec, title: t.title.clone(), lang, default: t.default, forced: false });
                } else {
                    let label = format!("a{ti}");
                    filters.push(format!("[0:a:{}]{}[{label}]", t.stream, chain.join(",")));
                    aouts.push(Out { map: format!("[{label}]"), codec: encode_args(&t.codec, out_i), title: t.title.clone(), lang, default: t.default, forced: false });
                }
            }
            Source::File(f) => {
                let delay_pos = t.delay_ms.max(0);
                let diff = match (vdur, t.duration) {
                    (Some(v), Some(d)) => d + t.delay_ms as f64 / 1000.0 - v,
                    _ => 0.0,
                };
                let fit = effective_fit(t.fit, diff);
                let mut fit_needs_vdur = false;
                let trim = fit == Fit::Trim && diff > 0.0;
                let pad = fit == Fit::Pad && diff < 0.0;
                if (fit == Fit::Trim || fit == Fit::Pad) && vdur.is_none() {
                    fit_needs_vdur = true;
                }
                if fit_needs_vdur {
                    warnings.push(format!("{what}: the video's length is unknown, so the track can't be trimmed or padded."));
                }
                let filtered = !chain.is_empty() || pad;
                let mut opts: Vec<String> = vec![];
                if t.delay_ms < 0 {
                    opts.extend(["-ss".into(), secs(-t.delay_ms)]);
                } else if t.delay_ms > 0 && !filtered {
                    opts.extend(["-itsoffset".into(), secs(t.delay_ms)]);
                }
                if trim {
                    if let Some(v) = vdur {
                        opts.extend(["-t".into(), fmt_num(((v - delay_pos as f64 / 1000.0) * 1000.0).round() / 1000.0)]);
                    }
                }
                let ii = input_index(&mut inputs, f, opts.clone());
                if filtered {
                    let mut ch = chain.clone();
                    if t.delay_ms > 0 {
                        ch.push(format!("adelay={}:all=1", t.delay_ms));
                    }
                    if pad {
                        if let Some(v) = vdur {
                            ch.push(format!("apad=whole_dur={}", fmt_num(v)));
                        }
                    }
                    let label = format!("a{ti}");
                    filters.push(format!("[{ii}:a:{}]{}[{label}]", t.stream, ch.join(",")));
                    aouts.push(Out { map: format!("[{label}]"), codec: encode_args(&t.codec, out_i), title: t.title.clone(), lang: lang.clone(), default: t.default, forced: false });
                } else {
                    let codec = if audio_fits(c, &t.codec) { vec![format!("-c:a:{out_i}"), "copy".into()] } else { encode_args(&t.codec, out_i) };
                    aouts.push(Out { map: format!("{ii}:a:{}", t.stream), codec, title: t.title.clone(), lang: lang.clone(), default: t.default, forced: false });
                }
                if t.mix_with_main {
                    match main_ref {
                        None => warnings.push(format!("{what}: “mix into the main audio” needs an original audio track that is kept; no mixed track was added.")),
                        Some(m) => {
                            // own input without -itsoffset (filters ignore it); the delay becomes adelay
                            let mut mopts: Vec<String> = vec![];
                            if t.delay_ms < 0 {
                                mopts.extend(["-ss".into(), secs(-t.delay_ms)]);
                            }
                            let mi = input_index(&mut inputs, f, mopts);
                            let mut ch = chain.clone();
                            if t.delay_ms > 0 {
                                ch.push(format!("adelay={}:all=1", t.delay_ms));
                            }
                            let lbl = format!("mix{ti}");
                            let pre = if ch.is_empty() { format!("[{mi}:a:{}]anull[c{ti}]", t.stream) } else { format!("[{mi}:a:{}]{}[c{ti}]", t.stream, ch.join(",")) };
                            filters.push(pre);
                            filters.push(format!("[0:a:{m}][c{ti}]amix=inputs=2:duration=first:dropout_transition=0:normalize=0[{lbl}]"));
                            let main_t = pr.audio.iter().find(|x| x.keep && x.source == Source::Main).unwrap();
                            let mtitle = format!("{} + {}", track_name(&main_t.title, &main_t.language, "Audio", m + 1), track_name(&t.title, &t.language, "Audio", n_audio));
                            let mo = aouts.len();
                            aouts.push(Out { map: format!("[{lbl}]"), codec: vec![format!("-c:a:{mo}"), "aac".into(), format!("-b:a:{mo}"), format!("{}k", pr.aac_bitrate_k)], title: mtitle, lang: lang_of(&main_t.language, &what, &mut vec![]), default: false, forced: false });
                        }
                    }
                }
            }
        }
    }

    let mut n_sub = 0usize;
    for t in pr.subs.iter().filter(|t| t.keep) {
        n_sub += 1;
        let what = format!("Subtitle track {n_sub}");
        let text = is_text_sub(&t.codec);
        if c == Container::Mp4 && !text {
            warnings.push(format!(
                "{what} ({}) is a picture subtitle. MP4 can't hold picture subtitles, so it is left out. Choose MKV to keep it.",
                sub_codec_name(&t.codec)
            ));
            continue;
        }
        let lang = lang_of(&t.language, &what, &mut warnings);
        let charset = effective_charset(t);
        let i = souts.len();
        let codec = match c {
            Container::Mp4 => {
                if matches!(t.codec.as_str(), "ass" | "ssa") {
                    warnings.push(format!("{what}: styled ASS/SSA subtitles become plain MP4 text (colours, fonts and positions are lost). Choose MKV to keep the styling."));
                }
                "mov_text"
            }
            Container::Mkv => {
                if !text {
                    "copy"
                } else if charset.is_empty() && matches!(t.codec.as_str(), "subrip" | "ass" | "ssa") {
                    "copy"
                } else if matches!(t.codec.as_str(), "ass" | "ssa") {
                    "ass"
                } else {
                    "srt"
                }
            }
        };
        let map = match &t.source {
            Source::Main => {
                if t.delay_ms != 0 {
                    warnings.push(format!("{what}: a delay only applies to added subtitles; ignored for the video's own track."));
                }
                format!("0:s:{}", t.stream)
            }
            Source::File(f) => {
                let mut opts: Vec<String> = vec![];
                if !charset.is_empty() {
                    opts.extend(["-sub_charenc".into(), charset.clone()]);
                }
                if t.delay_ms < 0 {
                    opts.extend(["-ss".into(), secs(-t.delay_ms)]);
                } else if t.delay_ms > 0 {
                    opts.extend(["-itsoffset".into(), secs(t.delay_ms)]);
                }
                let ii = input_index(&mut inputs, f, opts);
                format!("{ii}:s:{}", t.stream)
            }
        };
        souts.push(Out { map, codec: vec![format!("-c:s:{i}"), codec.into()], title: t.title.clone(), lang, default: t.default, forced: t.forced });
    }

    // ---- assemble
    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(&pr.video)];
    for inp in &inputs {
        a.extend(inp.opts.iter().cloned());
        a.push("-i".into());
        a.push(p(&inp.path));
    }
    if !filters.is_empty() {
        a.push("-filter_complex".into());
        a.push(filters.join(";"));
    }
    a.push_all(["-map", "0:V?"]);
    for o in aouts.iter().chain(souts.iter()) {
        a.push("-map".into());
        a.push(o.map.clone());
    }
    if c == Container::Mkv {
        a.push_all(["-map", "0:t?"]);
    }
    a.push_all(["-map_metadata", "0", "-map_chapters", "0", "-c:v", "copy"]);
    if c == Container::Mkv {
        a.push_all(["-c:t", "copy"]);
    }
    let first_default = |v: &[Out]| v.iter().position(|o| o.default);
    let (da, ds) = (first_default(&aouts), first_default(&souts));
    for (kind, outs, def) in [("a", &aouts, da), ("s", &souts, ds)] {
        for (i, o) in outs.iter().enumerate() {
            a.extend(o.codec.iter().cloned());
            a.push(format!("-metadata:s:{kind}:{i}"));
            a.push(format!("title={}", o.title.trim()));
            if c == Container::Mp4 {
                // most players show the MP4 track name from the handler, not the title tag
                a.push(format!("-metadata:s:{kind}:{i}"));
                a.push(format!("handler_name={}", o.title.trim()));
            }
            a.push(format!("-metadata:s:{kind}:{i}"));
            a.push(format!("language={}", o.lang));
            let mut disp: Vec<&str> = vec![];
            if def == Some(i) {
                disp.push("default");
            }
            if o.forced {
                disp.push("forced");
            }
            a.push(format!("-disposition:{kind}:{i}"));
            a.push(if disp.is_empty() { "0".into() } else { disp.join("+") });
        }
    }
    if c == Container::Mp4 && pr.faststart {
        a.push_all(["-movflags", "+faststart"]);
    }
    a.push(p(&pr.output));
    if aouts.is_empty() {
        warnings.push("The new file will have no audio.".into());
    }
    Ok(BuiltCommand { args: a, side_files: vec![], warnings })
}

// ---------------------------------------------------------------- extract jobs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioFormat {
    #[default]
    Wav,
    Flac,
    Mp3,
    M4a,
    Opus,
    /// Copy the original codec (no quality change).
    Original,
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 6] = [AudioFormat::Wav, AudioFormat::Flac, AudioFormat::Mp3, AudioFormat::M4a, AudioFormat::Opus, AudioFormat::Original];
    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Wav => "WAV (uncompressed, for editing)",
            AudioFormat::Flac => "FLAC (lossless, smaller than WAV)",
            AudioFormat::Mp3 => "MP3 (plays everywhere)",
            AudioFormat::M4a => "AAC / M4A",
            AudioFormat::Opus => "Opus (small, good quality)",
            AudioFormat::Original => "Original – copy as it is, no quality change",
        }
    }
}

/// File extension for an extracted audio track.
pub fn audio_ext(f: AudioFormat, codec: &str) -> &'static str {
    match f {
        AudioFormat::Wav => "wav",
        AudioFormat::Flac => "flac",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::M4a => "m4a",
        AudioFormat::Opus => "opus",
        AudioFormat::Original => match codec {
            "aac" | "alac" => "m4a",
            "mp3" => "mp3",
            "ac3" => "ac3",
            "eac3" => "eac3",
            "flac" => "flac",
            "opus" => "opus",
            "vorbis" => "ogg",
            "dts" => "dts",
            c if c.starts_with("pcm_") => "wav",
            _ => "mka",
        },
    }
}

/// Characters Windows doesn't allow in file names -> "_".
pub fn file_part(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    t.trim_end_matches(['.', ' ']).to_string()
}

/// "My video - audio 2 (eng, Commentary).wav"
pub fn extract_name(video: &Path, kind: &str, n: usize, language: &str, title: &str, ext: &str) -> String {
    let mut extra: Vec<String> = vec![];
    if !language.trim().is_empty() && language.trim() != "und" {
        extra.push(file_part(language));
    }
    if !title.trim().is_empty() {
        extra.push(file_part(title));
    }
    let extra = if extra.is_empty() { String::new() } else { format!(" ({})", extra.join(", ")) };
    format!("{} - {kind} {n}{extra}.{ext}", crate::util::stem(video))
}

pub fn extract_audio(video: &Path, stream: usize, codec: &str, f: AudioFormat, bitrate_k: u32, output: &Path) -> BuiltCommand {
    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(video), "-map".into(), format!("0:a:{stream}"), "-vn".into(), "-sn".into(), "-dn".into()];
    match f {
        AudioFormat::Wav => a.push_all(["-c:a", "pcm_s16le"]),
        AudioFormat::Flac => a.push_all(["-c:a", "flac"]),
        AudioFormat::Mp3 => a.push_all(["-c:a", "libmp3lame", "-q:a", "2"]),
        AudioFormat::M4a => {
            a.push_all(["-c:a", "aac", "-b:a"]);
            a.push(format!("{bitrate_k}k"));
        }
        AudioFormat::Opus => {
            a.push_all(["-c:a", "libopus", "-b:a", "160k"]);
        }
        AudioFormat::Original => a.push_all(["-c:a", "copy"]),
    }
    let _ = codec;
    a.push(p(output));
    BuiltCommand { args: a, ..Default::default() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubFormat {
    #[default]
    Srt,
    Ass,
    Original,
}

impl SubFormat {
    pub const ALL: [SubFormat; 3] = [SubFormat::Srt, SubFormat::Ass, SubFormat::Original];
    pub fn label(self) -> &'static str {
        match self {
            SubFormat::Srt => "SRT (plain text, works everywhere)",
            SubFormat::Ass => "ASS (keeps styling)",
            SubFormat::Original => "Original format (picture subtitles: .sup / .mks)",
        }
    }
}

/// (extension, ffmpeg subtitle codec) for an extracted subtitle track.
pub fn sub_target(f: SubFormat, codec: &str) -> Result<(&'static str, &'static str), String> {
    let text = is_text_sub(codec);
    match f {
        SubFormat::Srt | SubFormat::Ass if !text => Err(format!(
            "{} is a picture subtitle; it can't be turned into text. Choose “Original format” to save it as it is.",
            sub_codec_name(codec)
        )),
        SubFormat::Srt => Ok(("srt", "srt")),
        SubFormat::Ass => Ok(("ass", "ass")),
        SubFormat::Original => Ok(match codec {
            "subrip" | "srt" => ("srt", "copy"),
            "ass" => ("ass", "copy"),
            "ssa" => ("ssa", "copy"),
            "webvtt" => ("vtt", "copy"),
            "hdmv_pgs_subtitle" => ("sup", "copy"),
            c if is_text_sub(c) => ("srt", "srt"),
            _ => ("mks", "copy"),
        }),
    }
}

pub fn extract_sub(video: &Path, stream: usize, codec: &str, f: SubFormat, output: &Path) -> Result<BuiltCommand, String> {
    let (_, c) = sub_target(f, codec)?;
    let a: Vec<String> = vec!["-y".into(), "-i".into(), p(video), "-map".into(), format!("0:s:{stream}"), "-c:s".into(), c.into(), p(output)];
    Ok(BuiltCommand { args: a, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_a(stream: usize, codec: &str, title: &str, lang: &str) -> AudioTrack {
        AudioTrack { stream, codec: codec.into(), title: title.into(), language: lang.into(), ..Default::default() }
    }
    fn ext_a(file: &str, codec: &str, dur: f64, title: &str) -> AudioTrack {
        AudioTrack { source: Source::File(file.into()), codec: codec.into(), duration: Some(dur), title: title.into(), language: "eng".into(), ..Default::default() }
    }
    fn main_s(stream: usize, codec: &str, lang: &str) -> SubTrack {
        SubTrack { stream, codec: codec.into(), language: lang.into(), ..Default::default() }
    }
    fn ext_s(file: &str, codec: &str) -> SubTrack {
        SubTrack { source: Source::File(file.into()), codec: codec.into(), language: "eng".into(), charset: "UTF-8".into(), ..Default::default() }
    }
    fn params(out: &str, audio: Vec<AudioTrack>, subs: Vec<SubTrack>) -> TracksParams {
        TracksParams {
            video: "/v/in.mkv".into(),
            video_duration: Some(60.0),
            audio,
            subs,
            output: out.into(),
            aac_bitrate_k: 192,
            advanced: CompressorParams::default(),
            tolerance_s: 1.5,
            faststart: true,
        }
    }
    fn j(b: &BuiltCommand) -> String {
        b.args.join(" ")
    }

    #[test]
    fn keep_everything_mkv_copies() {
        let pr = params("/o/out.mkv", vec![main_a(0, "aac", "Game", "eng"), main_a(1, "opus", "Mic", "eng")], vec![main_s(0, "subrip", "eng"), main_s(1, "hdmv_pgs_subtitle", "ger")]);
        let b = build(&pr).unwrap();
        assert_eq!(
            j(&b),
            "-y -i /v/in.mkv -map 0:V? -map 0:a:0 -map 0:a:1 -map 0:s:0 -map 0:s:1 -map 0:t? -map_metadata 0 -map_chapters 0 -c:v copy -c:t copy \
             -c:a:0 copy -metadata:s:a:0 title=Game -metadata:s:a:0 language=eng -disposition:a:0 0 \
             -c:a:1 copy -metadata:s:a:1 title=Mic -metadata:s:a:1 language=eng -disposition:a:1 0 \
             -c:s:0 copy -metadata:s:s:0 title= -metadata:s:s:0 language=eng -disposition:s:0 0 \
             -c:s:1 copy -metadata:s:s:1 title= -metadata:s:s:1 language=ger -disposition:s:1 0 /o/out.mkv"
        );
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    }

    #[test]
    fn remove_reorder_default_and_mp4_rules() {
        // order = output order: Mic first; Game removed; opus can't go into MP4 -> AAC; handler_name for titles
        let mut game = main_a(0, "aac", "Game", "eng");
        game.keep = false;
        let mut mic = main_a(1, "opus", "Mic", "en");
        mic.default = true;
        let mut forced = main_s(0, "subrip", "English");
        forced.forced = true;
        forced.default = true;
        let pr = params("/o/out.mp4", vec![mic, game], vec![forced, main_s(1, "hdmv_pgs_subtitle", "ger"), main_s(2, "ass", "jpn")]);
        let b = build(&pr).unwrap();
        let s = j(&b);
        assert!(s.contains("-map 0:V? -map 0:a:1 -map 0:s:0 -map 0:s:2 -map_metadata 0"), "{s}");
        assert!(!s.contains("0:a:0") && !s.contains("0:t?") && !s.contains("0:s:1"), "{s}");
        assert!(s.contains("-c:a:0 aac -b:a:0 192k -metadata:s:a:0 title=Mic -metadata:s:a:0 handler_name=Mic -metadata:s:a:0 language=eng -disposition:a:0 default"), "{s}");
        assert!(s.contains("-c:s:0 mov_text -metadata:s:s:0 title= -metadata:s:s:0 handler_name= -metadata:s:s:0 language=eng -disposition:s:0 default+forced"), "{s}");
        assert!(s.contains("-c:s:1 mov_text") && s.contains("language=jpn -disposition:s:1 0"), "{s}");
        assert!(s.ends_with("-movflags +faststart /o/out.mp4"));
        assert_eq!(b.warnings.len(), 2, "{:?}", b.warnings);
        assert!(b.warnings[0].contains("picture subtitle") && b.warnings[1].contains("styled ASS"));
        // only the first default wins
        let mut a1 = main_a(0, "aac", "", "");
        a1.default = true;
        let mut a2 = main_a(1, "aac", "", "");
        a2.default = true;
        let s = j(&build(&params("/o/o.mkv", vec![a1, a2], vec![])).unwrap());
        assert!(s.contains("-disposition:a:0 default") && s.contains("-disposition:a:1 0"));
    }

    #[test]
    fn add_commentary_with_delay_gain_and_mix() {
        let main = main_a(0, "aac", "Game", "eng");
        let mut com = ext_a("/a/com.wav", "pcm_s16le", 60.4, "Commentary Track");
        com.delay_ms = 250;
        let pr = params("/o/out.mkv", vec![main.clone(), com.clone()], vec![]);
        let s = j(&build(&pr).unwrap());
        // copy into MKV (WAV is fine there), delayed with -itsoffset, within tolerance -> trimmed by Auto (longer)
        assert!(s.starts_with("-y -i /v/in.mkv -itsoffset 0.25 -t 59.75 -i /a/com.wav -map 0:V? -map 0:a:0 -map 1:a:0"), "{s}");
        assert!(s.contains("-c:a:1 copy -metadata:s:a:1 title=Commentary Track"), "{s}");
        // MP4: WAV must be encoded to AAC
        let s = j(&build(&params("/o/out.mp4", vec![main.clone(), com.clone()], vec![])).unwrap());
        assert!(s.contains("-c:a:1 aac -b:a:1 192k"), "{s}");
        // gain + compressor -> filter graph with adelay instead of itsoffset; lossless source in MKV -> FLAC
        com.gain_db = 3.0;
        com.compressor = CompressorPreset::Medium;
        com.mix_with_main = true;
        let b = build(&params("/o/out.mkv", vec![main, com], vec![])).unwrap();
        let s = j(&b);
        assert!(s.contains("-t 59.75 -i /a/com.wav -i /a/com.wav -filter_complex [1:a:0]volume=3dB,acompressor="), "{s}");
        assert!(s.contains(",adelay=250:all=1[a1];[2:a:0]volume=3dB,acompressor="), "{s}");
        assert!(s.contains("[0:a:0][c1]amix=inputs=2:duration=first:dropout_transition=0:normalize=0[mix1]"), "{s}");
        assert!(s.contains("-map 0:a:0 -map [a1] -map [mix1]"), "{s}");
        assert!(s.contains("-c:a:1 flac") && s.contains("-c:a:2 aac -b:a:2 192k -metadata:s:a:2 title=Game + Commentary Track"), "{s}");
        assert_eq!(summary(&params("/o/out.mkv", vec![main_a(0, "aac", "Game", "eng"), { let mut c = ext_a("/a/c.wav", "pcm_s16le", 60.0, "Commentary"); c.mix_with_main = true; c }], vec![main_s(0, "subrip", "eng")]), 1),
            "Output will have: 1 video, 3 audio (Game, Commentary, Game + Commentary), 1 subtitle (English)");
    }

    #[test]
    fn replace_audio_and_lengths() {
        // replace = original unchecked + new track in its place
        let mut orig = main_a(0, "aac", "Game", "eng");
        orig.keep = false;
        let mut new = ext_a("/a/new.flac", "flac", 57.0, "Game");
        new.default = true;
        // shorter by 3 s: Auto leaves it; Pad fills with silence
        let s = j(&build(&params("/o/out.mp4", vec![new.clone(), orig.clone()], vec![])).unwrap());
        assert!(s.starts_with("-y -i /v/in.mkv -i /a/new.flac -map 0:V? -map 1:a:0 -map_metadata"), "{s}");
        assert!(s.contains("-c:a:0 aac") && !s.contains("-t ") && !s.contains("apad"), "{s}");
        new.fit = Fit::Pad;
        let s = j(&build(&params("/o/out.mkv", vec![new.clone(), orig.clone()], vec![])).unwrap());
        assert!(s.contains("-filter_complex [1:a:0]apad=whole_dur=60[a0]") && s.contains("-c:a:0 flac"), "{s}");
        // longer, keep full: nothing trimmed
        new.duration = Some(64.0);
        new.fit = Fit::KeepFull;
        let s = j(&build(&params("/o/out.mkv", vec![new.clone()], vec![])).unwrap());
        assert!(!s.contains("-t ") && s.contains("-c:a:0 copy"), "{s}");
        new.fit = Fit::Auto;
        let s = j(&build(&params("/o/out.mkv", vec![new.clone()], vec![])).unwrap());
        assert!(s.contains("-t 60 -i /a/new.flac"), "{s}");
        // negative delay skips the start of the file
        new.delay_ms = -1500;
        let s = j(&build(&params("/o/out.mkv", vec![new], vec![])).unwrap());
        assert!(s.contains("-ss 1.5 -t 60 -i /a/new.flac"), "{s}");
        // length notes / tolerance
        assert_eq!(length_check(60.0, 61.0, 0, 1.5).1, Length::Matches);
        assert_eq!(length_check(60.0, 62.0, 0, 1.5).1, Length::Longer);
        assert_eq!(length_check(60.0, 58.0, 0, 1.5).1, Length::Shorter);
        assert_eq!(length_check(60.0, 58.0, 2000, 1.5).1, Length::Matches);
        assert!(length_note(60.0, 61.0, 0, 1.5).contains("close enough"));
        assert!(length_note(60.0, 55.0, 0, 1.5).contains("5.0 s shorter"));
        assert_eq!(effective_fit(Fit::Auto, 0.5), Fit::Trim);
        assert_eq!(effective_fit(Fit::Auto, -3.0), Fit::Leave);
        assert_eq!(effective_fit(Fit::Pad, 2.0), Fit::Leave);
        // no audio at all is allowed, with a note
        let b = build(&params("/o/out.mkv", vec![orig], vec![])).unwrap();
        assert!(b.warnings.iter().any(|w| w.contains("no audio")));
    }

    #[test]
    fn external_subtitles() {
        let mut srt = ext_s("/s/a.srt", "subrip");
        srt.delay_ms = -500;
        srt.forced = true;
        let mut latin = ext_s("/s/latin1.srt", "subrip");
        latin.charset = "CP1252".into();
        latin.language = "fr".into();
        let ass = ext_s("/s/b.ass", "ass");
        let vtt = ext_s("/s/c.vtt", "webvtt");
        let sup = ext_s("/s/d.sup", "hdmv_pgs_subtitle");
        let subs = vec![srt, latin, ass, vtt, sup];
        let s = j(&build(&params("/o/out.mkv", vec![], subs.clone())).unwrap());
        assert!(s.contains("-ss 0.5 -i /s/a.srt -sub_charenc CP1252 -i /s/latin1.srt -i /s/b.ass -i /s/c.vtt -i /s/d.sup"), "{s}");
        assert!(s.contains("-map 1:s:0 -map 2:s:0 -map 3:s:0 -map 4:s:0 -map 5:s:0"), "{s}");
        // MKV: utf-8 srt/ass copied, re-coded srt converted, vtt -> srt, picture copied
        for (i, c) in ["copy", "srt", "copy", "srt", "copy"].iter().enumerate() {
            assert!(s.contains(&format!("-c:s:{i} {c} ")), "{i} {c}: {s}");
        }
        assert!(s.contains("-metadata:s:s:1 language=fre") && s.contains("-disposition:s:0 forced"), "{s}");
        // MP4: text -> mov_text, the picture track is left out with a warning
        let b = build(&params("/o/out.mp4", vec![], subs)).unwrap();
        let s = j(&b);
        assert!(s.contains("-c:s:3 mov_text") && !s.contains("-c:s:4") && !s.contains("5:s:0"), "{s}");
        assert!(b.warnings.iter().any(|w| w.contains("picture subtitle")));
        // unknown language -> warning, stored as und
        let mut x = ext_s("/s/a.srt", "subrip");
        x.language = "klingonish".into();
        let b = build(&params("/o/out.mkv", vec![], vec![x])).unwrap();
        assert!(j(&b).contains("language=und") && b.warnings[0].contains("not a language code"));
    }

    #[test]
    fn charset_detection() {
        assert_eq!(detect_charset("Grüße".as_bytes()), None);
        assert_eq!(detect_charset(&[0xEF, 0xBB, 0xBF, b'a']), None);
        assert_eq!(detect_charset(&[0xFF, 0xFE, b'a', 0]), None);
        assert_eq!(detect_charset(b"Gr\xfc\xdfe"), Some("CP1252"));
        assert_eq!(normalize_language("EN").as_deref(), Some("eng"));
        assert_eq!(normalize_language("German").as_deref(), Some("ger"));
        assert_eq!(normalize_language("").as_deref(), Some("und"));
        assert_eq!(normalize_language("tlh").as_deref(), Some("tlh"));
        assert_eq!(normalize_language("e n"), None);
    }

    #[test]
    fn extract_formats_and_names() {
        let v = Path::new("/v/My Video.mkv");
        assert_eq!(extract_name(v, "audio", 2, "eng", "Commentary: Bob", "wav"), "My Video - audio 2 (eng, Commentary_ Bob).wav");
        assert_eq!(extract_name(v, "subtitles", 1, "und", "", "srt"), "My Video - subtitles 1.srt");
        let e = |f| j(&extract_audio(v, 1, "aac", f, 192, Path::new("/o/x")));
        assert_eq!(e(AudioFormat::Wav), "-y -i /v/My Video.mkv -map 0:a:1 -vn -sn -dn -c:a pcm_s16le /o/x");
        assert!(e(AudioFormat::Flac).contains("-c:a flac"));
        assert!(e(AudioFormat::Mp3).contains("-c:a libmp3lame -q:a 2"));
        assert!(e(AudioFormat::M4a).contains("-c:a aac -b:a 192k"));
        assert!(e(AudioFormat::Opus).contains("-c:a libopus -b:a 160k"));
        assert!(e(AudioFormat::Original).contains("-c:a copy"));
        assert_eq!(audio_ext(AudioFormat::Original, "aac"), "m4a");
        assert_eq!(audio_ext(AudioFormat::Original, "pcm_s24le"), "wav");
        assert_eq!(audio_ext(AudioFormat::Original, "truehd"), "mka");
        assert_eq!(sub_target(SubFormat::Original, "mov_text").unwrap(), ("srt", "srt"));
        assert_eq!(sub_target(SubFormat::Original, "hdmv_pgs_subtitle").unwrap(), ("sup", "copy"));
        assert_eq!(sub_target(SubFormat::Ass, "subrip").unwrap(), ("ass", "ass"));
        assert!(sub_target(SubFormat::Srt, "dvd_subtitle").is_err());
        assert_eq!(j(&extract_sub(v, 0, "subrip", SubFormat::Srt, Path::new("/o/s.srt")).unwrap()), "-y -i /v/My Video.mkv -map 0:s:0 -c:s srt /o/s.srt");
        assert!(build(&params("/o/x.avi", vec![], vec![])).is_err());
    }
}
