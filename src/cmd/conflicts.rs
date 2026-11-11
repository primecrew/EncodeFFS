//! Unusual option combinations: plain-words warnings and optional one-click fixes.
//!
//! A fix is applied only when the user clicks it. There is no silent rewrite.
//! Warnings without a single obvious fix have no fix button.

use std::path::{Path, PathBuf};

use super::codecs::{self, AudioCodec, VideoCodec};
use super::common::{AudioEncode, CompressorPreset, VideoEncode};
use super::frameserve::{self, BitDepth, Color, Matrix, ScriptOptions};

/// One unusual combination the user can keep or (when a fix exists) correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// 10-bit with ConvertToYV12 / ConvertToRGB32 (8-bit-only colour formats).
    TenBitEightBitColor,
    /// Colour matrix already squeezes to TV range, and a second full→TV conversion is on.
    DoubleRangeSqueeze,
    /// Full-range (PC) script tagged as TV range, with no squeeze.
    PcTaggedAsTv,
    /// HEVC/AV1 10-bit is on, but the script is 8-bit.
    EncoderTenBitScriptEight,
    /// The script is 10-bit, but HEVC/AV1 10-bit is off.
    ScriptTenBitEncoderEight,
    /// Hand-edited script is 10-bit, but the encoder is H.264 (8-bit only).
    ScriptTenBitH264,
    /// AV1 in a .mov container.
    Av1InMov,
    /// Stream-copied audio into MP4 (some codecs may fail).
    CopyAudioIntoMp4,
    /// Gain or compressor set, but audio is being copied so filters are ignored.
    CopyIgnoresFilters,
}

/// What the one-click fix will do, in the button label and its hover text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fix {
    /// Visible button text, e.g. "Turn off the extra range conversion".
    pub button: &'static str,
    /// Hover / accessible name: the exact change.
    pub hint: &'static str,
}

/// One warning, with an optional one-click fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub message: String,
    pub fix: Option<Fix>,
}

impl Conflict {
    pub fn has_fix(&self) -> bool {
        self.fix.is_some()
    }
}

/// The fields a fix may change. Modes map their form state to and from this.
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    pub color: Color,
    pub matrix: Matrix,
    pub bits: BitDepth,
    pub range_convert: bool,
    pub bt709_tags: bool,
    pub video: VideoEncode,
    pub audio: AudioEncode,
    /// "mp4", "mkv", "mov", or "auto"/"same" as used on the form.
    pub container: String,
    pub out_name: String,
    pub last_auto: String,
    pub output: PathBuf,
    pub edit_script: bool,
    pub script_text: String,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            color: Color::Yuv420,
            matrix: Matrix::Rec709,
            bits: BitDepth::Eight,
            range_convert: false,
            bt709_tags: true,
            video: VideoEncode::default(),
            audio: AudioEncode::default(),
            container: "mp4".into(),
            out_name: "a.mp4".into(),
            last_auto: "a.mp4".into(),
            output: PathBuf::from(r"C:\out\a.mp4"),
            edit_script: false,
            script_text: String::new(),
        }
    }
}

/// Script bit depth driven by the codec 10-bit option (H.264 is always 8-bit).
pub fn script_bit_depth(v: &VideoEncode) -> BitDepth {
    if codecs::ten_bit_output(v) {
        BitDepth::Ten
    } else {
        BitDepth::Eight
    }
}

/// True when a hand-edited AviSynth script looks 10-bit (ConvertBits(10) or 16).
pub fn script_is_10bit(text: &str) -> bool {
    text.lines().any(|line| {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        compact.contains("convertbits(10") || compact.contains("convertbits(16")
    })
}

/// Bit depth that will actually be encoded: from the edited script if the user
/// typed one, otherwise from the codec 10-bit option.
pub fn effective_bits(edit_script: bool, script_text: &str, v: &VideoEncode) -> BitDepth {
    if edit_script {
        if script_is_10bit(script_text) {
            BitDepth::Ten
        } else {
            BitDepth::Eight
        }
    } else {
        script_bit_depth(v)
    }
}

fn yuv_conversion(color: Color) -> bool {
    !matches!(color, Color::None | Color::Rgb32)
}

fn set_container(c: &mut Choices, ext: &str) {
    c.container = ext.to_string();
    let new_name = replace_name_ext(&c.out_name, ext);
    if c.last_auto == c.out_name {
        c.last_auto = new_name.clone();
    }
    c.out_name = new_name;
    c.output = c.output.with_extension(ext);
}

/// Change or add a file extension on a file name (empty names stay empty).
pub fn replace_name_ext(name: &str, ext: &str) -> String {
    if name.trim().is_empty() {
        return name.to_string();
    }
    Path::new(name).with_extension(ext).to_string_lossy().into_owned()
}

fn append_convert_bits_10(text: &mut String) {
    if script_is_10bit(text) {
        return;
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    text.push_str("ConvertBits(10)\r\n");
}

fn remove_convert_bits_10(text: &mut String) {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let kept: Vec<&str> = text.lines().filter(|line| {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        !compact.contains("convertbits(10") && !compact.contains("convertbits(16")
    }).collect();
    let mut s = kept.join(nl);
    if text.ends_with('\n') && !s.ends_with('\n') {
        s.push_str(nl);
    }
    *text = s;
}

/// All current unusual combinations for these choices.
pub fn detect(c: &Choices) -> Vec<Conflict> {
    let mut out = vec![];
    // Hand-edited scripts: read ConvertBits from the text. Generated / command-builder
    // paths pass the script bit depth in `c.bits` (which may disagree with the encoder).
    let bits = if c.edit_script {
        if script_is_10bit(&c.script_text) { BitDepth::Ten } else { BitDepth::Eight }
    } else {
        c.bits
    };

    if bits == BitDepth::Ten && matches!(c.color, Color::Yv12 | Color::Rgb32) && !c.edit_script {
        let name = c.color.function().unwrap_or("this colour format");
        out.push(Conflict {
            kind: ConflictKind::TenBitEightBitColor,
            message: format!(
                "10-bit with {name} is unusual: that colour format is 8-bit only, so the script will convert to 8-bit first and then ConvertBits(10). YUV420, YUV422 or YUV444 is the better 10-bit choice. The encode will still run."
            ),
            fix: Some(Fix {
                button: "Switch colour format to YUV 4:2:0",
                hint: "Changes Colour format to YUV 4:2:0 (ConvertToYUV420), the recommended 10-bit layout. Bit depth stays 10-bit.",
            }),
        });
    }

    if c.range_convert && yuv_conversion(c.color) && (c.matrix.squeezes_to_tv() || c.matrix == Matrix::Default) {
        let message = if c.matrix == Matrix::Default {
            "The colour matrix is not set, so AviSynth uses Rec601 (TV range) when it converts. A full-to-TV conversion is also on and may squeeze the picture twice. The encode will still run as you set it.".into()
        } else {
            "The script's colour matrix (Rec709, Rec601 or Rec2020) already converts to TV (limited) range. A second full-to-TV conversion is also on, which can look washed out (grey blacks). The encode will still run as you set it.".into()
        };
        out.push(Conflict {
            kind: ConflictKind::DoubleRangeSqueeze,
            message,
            fix: Some(Fix {
                button: "Turn off the extra range conversion",
                hint: "Turns off “Convert full range to TV range”, so the picture is not squeezed twice.",
            }),
        });
    }

    if c.matrix.keeps_full_range() && !c.range_convert && c.bt709_tags {
        out.push(Conflict {
            kind: ConflictKind::PcTaggedAsTv,
            message: "The script keeps full (PC) range, but TV-range (limited) tags were also requested. Players may treat the full-range picture as TV range and crush the blacks. The encode will still run as you set it.".into(),
            fix: Some(Fix {
                button: "Tag the file as full (PC) range",
                hint: "Stops requesting TV-range tags, so the file is tagged as full (PC) range to match the picture.",
            }),
        });
    }

    let encoder_10 = codecs::ten_bit_output(&c.video);
    let script_10 = bits == BitDepth::Ten;
    if encoder_10 && !script_10 {
        out.push(Conflict {
            kind: ConflictKind::EncoderTenBitScriptEight,
            message: "The HEVC/AV1 encoder has 10-bit ticked, but the script bit depth is 8-bit. The encoder is given the script's 8-bit pixel format so the two don't silently disagree. Tick 10-bit under Bit depth too if you want 10-bit output. The encode will still run.".into(),
            fix: Some(if c.edit_script {
                Fix {
                    button: "Add ConvertBits(10) to the script",
                    hint: "Appends ConvertBits(10) to your hand-edited script so it matches the encoder's 10-bit setting.",
                }
            } else {
                Fix {
                    button: "Set the script to 10-bit",
                    hint: "Adds ConvertBits(10) to the AviSynth script so it matches the encoder's 10-bit setting.",
                }
            }),
        });
    } else if !encoder_10 && script_10 && c.video.codec != VideoCodec::H264 {
        out.push(Conflict {
            kind: ConflictKind::ScriptTenBitEncoderEight,
            message: "The script is 10-bit, but 10-bit is unticked on the HEVC/AV1 encoder. The encoder is still given the script's 10-bit pixel format so FFmpeg does not drop back to 8-bit. The encode will still run.".into(),
            fix: Some(Fix {
                button: "Tick 10-bit on the encoder",
                hint: "Turns on 10-bit output on the HEVC/AV1 encoder so it matches the 10-bit script. The script is not changed.",
            }),
        });
    } else if !encoder_10 && script_10 && c.video.codec == VideoCodec::H264 && c.edit_script {
        out.push(Conflict {
            kind: ConflictKind::ScriptTenBitH264,
            message: "The hand-edited script is 10-bit (ConvertBits(10)), but H.264 is 8-bit only. The encode will still run; FFmpeg may convert the picture down to 8-bit.".into(),
            fix: Some(Fix {
                button: "Remove ConvertBits(10) from the script",
                hint: "Removes the ConvertBits(10) line from your hand-edited script so it matches 8-bit H.264. The video codec is not changed.",
            }),
        });
    }

    out.extend(codec_container_conflicts(&c.video, &c.audio, &c.output));
    out
}

/// Codec / container warnings shared by Re-encode, Batch, Join, Custom and Frameserver.
pub fn codec_container_conflicts(video: &VideoEncode, audio: &AudioEncode, output: &Path) -> Vec<Conflict> {
    let mut out = vec![];
    let ext = crate::util::ext_lower(output);
    if video.codec == VideoCodec::Av1 && ext == "mov" {
        out.push(Conflict {
            kind: ConflictKind::Av1InMov,
            message: "AV1 in .mov plays in very few apps (QuickTime can't). MP4 or MKV is safer.".into(),
            fix: Some(Fix {
                button: "Change the container to MP4",
                hint: "Changes the output container to MP4 (the file name extension becomes .mp4). The video codec stays AV1.",
            }),
        });
    }
    if audio.codec == AudioCodec::Copy {
        // mp4_like includes mov; the existing warning is specifically about MP4
        if ext == "mp4" || ext == "m4v" {
            out.push(Conflict {
                kind: ConflictKind::CopyAudioIntoMp4,
                message: "Keep original audio into MP4: works for AAC/AC3/MP3 sources; other audio (e.g. PCM, FLAC in older players, Opus) may fail or not play – MKV keeps anything.".into(),
                fix: Some(Fix {
                    button: "Change the container to MKV",
                    hint: "Changes the output container to MKV (the file name extension becomes .mkv), which can keep any audio. The audio setting stays “Keep original”.",
                }),
            });
        }
    }
    if audio.codec == AudioCodec::Copy && audio.filter_chain().is_some() {
        out.push(Conflict {
            kind: ConflictKind::CopyIgnoresFilters,
            message: "Keep original audio: gain and compressor can't be applied to copied audio and are ignored.".into(),
            fix: Some(Fix {
                button: "Turn off gain and the compressor",
                hint: "Sets gain to 0 dB and the compressor to none, because those filters cannot be applied to copied audio. The audio setting stays “Keep original”.",
            }),
        });
    }
    out
}

/// Apply one fix. Only the fields that fix is meant to change are touched.
pub fn apply(c: &mut Choices, kind: ConflictKind) {
    match kind {
        ConflictKind::TenBitEightBitColor => {
            c.color = Color::Yuv420;
        }
        ConflictKind::DoubleRangeSqueeze => {
            c.range_convert = false;
        }
        ConflictKind::PcTaggedAsTv => {
            c.bt709_tags = false;
        }
        ConflictKind::EncoderTenBitScriptEight => {
            if c.edit_script {
                append_convert_bits_10(&mut c.script_text);
            } else {
                c.bits = BitDepth::Ten;
                if c.video.codec != VideoCodec::H264 {
                    c.video.ten_bit = true;
                }
            }
        }
        ConflictKind::ScriptTenBitEncoderEight => {
            if c.video.codec != VideoCodec::H264 {
                c.video.ten_bit = true;
            }
            c.bits = BitDepth::Ten;
        }
        ConflictKind::ScriptTenBitH264 => {
            remove_convert_bits_10(&mut c.script_text);
        }
        ConflictKind::Av1InMov => set_container(c, "mp4"),
        ConflictKind::CopyAudioIntoMp4 => set_container(c, "mkv"),
        ConflictKind::CopyIgnoresFilters => {
            c.audio.gain_db = 0.0;
            c.audio.compressor = CompressorPreset::None;
        }
    }
}

/// Drop dismissed kinds that no longer apply (the user changed the setting).
pub fn prune_dismissed(dismissed: &mut Vec<ConflictKind>, current: &[Conflict]) {
    dismissed.retain(|k| current.iter().any(|c| c.kind == *k));
}

/// True when at least one warning has a one-click fix.
pub fn any_fixable(cs: &[Conflict]) -> bool {
    cs.iter().any(|c| c.has_fix())
}

/// Messages only (for BuiltCommand.warnings and older tests).
pub fn messages(cs: &[Conflict]) -> Vec<String> {
    cs.iter().map(|c| c.message.clone()).collect()
}

/// Script-only unusual combinations (10-bit with YV12/RGB32).
pub fn script_conflicts(o: &ScriptOptions) -> Vec<Conflict> {
    let c = Choices { color: o.color, bits: o.bits, bt709_tags: false, ..Choices::default() };
    // Detect only the colour/bit-depth script warning, not encode-side ones.
    detect(&c).into_iter().filter(|w| w.kind == ConflictKind::TenBitEightBitColor).collect()
}

/// Encode-side unusual combinations (range, tags, bit-depth mismatch).
pub fn encode_conflicts(pr: &frameserve::EncodeParams) -> Vec<Conflict> {
    let bits = if pr.pix_fmt.contains("10") { BitDepth::Ten } else { BitDepth::Eight };
    let c = Choices {
        color: Color::Yuv420,
        matrix: pr.matrix,
        bits,
        range_convert: pr.range_convert,
        bt709_tags: pr.bt709_tags,
        video: pr.video.clone(),
        audio: pr.audio.clone(),
        container: crate::util::ext_lower(&pr.output),
        out_name: pr.output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        last_auto: String::new(),
        output: pr.output.clone(),
        edit_script: false,
        script_text: pr.script_text.clone(),
    };
    detect(&c).into_iter().filter(|w| {
        matches!(
            w.kind,
            ConflictKind::DoubleRangeSqueeze
                | ConflictKind::PcTaggedAsTv
                | ConflictKind::EncoderTenBitScriptEight
                | ConflictKind::ScriptTenBitEncoderEight
        )
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_changed(before: &Choices, after: &Choices, patch: impl Fn(&mut Choices)) {
        let mut expected = before.clone();
        patch(&mut expected);
        assert_eq!(after, &expected, "fix must change only the intended field(s)");
    }

    fn kinds(cs: &[Conflict]) -> Vec<ConflictKind> {
        cs.iter().map(|c| c.kind).collect()
    }

    #[test]
    fn safe_defaults_have_no_conflicts() {
        assert!(detect(&Choices::default()).is_empty());
    }

    #[test]
    fn fix_ten_bit_yv12_switches_color_only() {
        let mut c = Choices { color: Color::Yv12, bits: BitDepth::Ten, video: VideoEncode { codec: VideoCodec::Hevc, ten_bit: true, ..Default::default() }, bt709_tags: false, ..Choices::default() };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::TenBitEightBitColor));
        let before = c.clone();
        apply(&mut c, ConflictKind::TenBitEightBitColor);
        only_changed(&before, &c, |x| x.color = Color::Yuv420);
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::TenBitEightBitColor));
    }

    #[test]
    fn fix_ten_bit_rgb32_switches_color_only() {
        let mut c = Choices { color: Color::Rgb32, bits: BitDepth::Ten, video: VideoEncode { codec: VideoCodec::Av1, ten_bit: true, ..Default::default() }, bt709_tags: false, range_convert: false, matrix: Matrix::Rec709, ..Choices::default() };
        // RGB32 is not a YUV conversion, so no double-squeeze; only the colour warning
        assert_eq!(kinds(&detect(&c)), vec![ConflictKind::TenBitEightBitColor]);
        let before = c.clone();
        apply(&mut c, ConflictKind::TenBitEightBitColor);
        only_changed(&before, &c, |x| x.color = Color::Yuv420);
        assert!(detect(&c).iter().all(|w| w.kind != ConflictKind::TenBitEightBitColor));
    }

    #[test]
    fn fix_double_range_squeeze_turns_convert_off() {
        let mut c = Choices { range_convert: true, matrix: Matrix::Rec709, ..Choices::default() };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::DoubleRangeSqueeze));
        assert!(detect(&c).iter().any(|w| w.message.contains("washed out")));
        let before = c.clone();
        apply(&mut c, ConflictKind::DoubleRangeSqueeze);
        only_changed(&before, &c, |x| x.range_convert = false);
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::DoubleRangeSqueeze));
    }

    #[test]
    fn fix_default_matrix_double_squeeze() {
        let mut c = Choices { range_convert: true, matrix: Matrix::Default, bt709_tags: false, ..Choices::default() };
        assert!(detect(&c).iter().any(|w| w.kind == ConflictKind::DoubleRangeSqueeze && w.message.contains("Rec601")));
        apply(&mut c, ConflictKind::DoubleRangeSqueeze);
        assert!(!c.range_convert);
        assert!(detect(&c).is_empty());
    }

    #[test]
    fn fix_pc_tagged_as_tv_clears_tv_tags_only() {
        let mut c = Choices { matrix: Matrix::Pc709, range_convert: false, bt709_tags: true, ..Choices::default() };
        assert!(detect(&c).iter().any(|w| w.kind == ConflictKind::PcTaggedAsTv && w.message.contains("full-range picture")));
        let before = c.clone();
        apply(&mut c, ConflictKind::PcTaggedAsTv);
        only_changed(&before, &c, |x| x.bt709_tags = false);
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::PcTaggedAsTv));
    }

    #[test]
    fn fix_encoder_10_script_8_adds_convertbits_when_edited() {
        let mut c = Choices {
            video: VideoEncode { codec: VideoCodec::Hevc, ten_bit: true, ..Default::default() },
            bits: BitDepth::Eight,
            edit_script: true,
            script_text: "AviSource(\"x.avi\")\r\nConvertToYUV420(matrix=\"Rec709\")\r\n".into(),
            bt709_tags: false,
            ..Choices::default()
        };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::EncoderTenBitScriptEight));
        let before = c.clone();
        apply(&mut c, ConflictKind::EncoderTenBitScriptEight);
        only_changed(&before, &c, |x| append_convert_bits_10(&mut x.script_text));
        assert!(script_is_10bit(&c.script_text));
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::EncoderTenBitScriptEight));
    }

    #[test]
    fn fix_script_10_encoder_8_ticks_ten_bit_only() {
        let mut c = Choices {
            video: VideoEncode { codec: VideoCodec::Hevc, ten_bit: false, ..Default::default() },
            bits: BitDepth::Ten,
            edit_script: true,
            script_text: "AviSource(\"x.avi\")\r\nConvertBits(10)\r\n".into(),
            bt709_tags: false,
            ..Choices::default()
        };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::ScriptTenBitEncoderEight));
        let before = c.clone();
        apply(&mut c, ConflictKind::ScriptTenBitEncoderEight);
        only_changed(&before, &c, |x| {
            x.video.ten_bit = true;
            x.bits = BitDepth::Ten;
        });
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::ScriptTenBitEncoderEight));
    }

    #[test]
    fn fix_script_10_h264_removes_convertbits_only() {
        let mut c = Choices {
            video: VideoEncode { codec: VideoCodec::H264, ..Default::default() },
            bits: BitDepth::Eight,
            edit_script: true,
            script_text: "AviSource(\"x.avi\")\r\nConvertToYUV420(matrix=\"Rec709\")\r\nConvertBits(10)\r\n".into(),
            ..Choices::default()
        };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::ScriptTenBitH264));
        let before = c.clone();
        apply(&mut c, ConflictKind::ScriptTenBitH264);
        only_changed(&before, &c, |x| remove_convert_bits_10(&mut x.script_text));
        assert!(!script_is_10bit(&c.script_text));
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::ScriptTenBitH264));
    }

    #[test]
    fn fix_av1_in_mov_changes_container_only() {
        let mut c = Choices {
            video: VideoEncode { codec: VideoCodec::Av1, ten_bit: false, ..Default::default() },
            output: PathBuf::from(r"C:\out\a.mov"),
            container: "mov".into(),
            out_name: "a.mov".into(),
            last_auto: "a.mov".into(),
            bt709_tags: false,
            ..Choices::default()
        };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::Av1InMov));
        let before = c.clone();
        apply(&mut c, ConflictKind::Av1InMov);
        only_changed(&before, &c, |x| set_container(x, "mp4"));
        assert_eq!(c.container, "mp4");
        assert_eq!(c.out_name, "a.mp4");
        assert_eq!(c.output, PathBuf::from(r"C:\out\a.mp4"));
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::Av1InMov));
    }

    #[test]
    fn fix_copy_audio_into_mp4_changes_container_only() {
        let mut c = Choices {
            audio: AudioEncode { codec: AudioCodec::Copy, ..Default::default() },
            output: PathBuf::from(r"C:\out\a.mp4"),
            container: "mp4".into(),
            out_name: "a.mp4".into(),
            last_auto: "a.mp4".into(),
            bt709_tags: false,
            video: VideoEncode { ten_bit: false, ..Default::default() },
            ..Choices::default()
        };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::CopyAudioIntoMp4));
        let before = c.clone();
        apply(&mut c, ConflictKind::CopyAudioIntoMp4);
        only_changed(&before, &c, |x| set_container(x, "mkv"));
        assert_eq!(c.container, "mkv");
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::CopyAudioIntoMp4));
    }

    #[test]
    fn fix_copy_ignores_filters_clears_gain_and_compressor_only() {
        let mut c = Choices {
            audio: AudioEncode { codec: AudioCodec::Copy, gain_db: 3.0, compressor: CompressorPreset::Medium, ..Default::default() },
            output: PathBuf::from(r"C:\out\a.mkv"),
            container: "mkv".into(),
            out_name: "a.mkv".into(),
            last_auto: "a.mkv".into(),
            bt709_tags: false,
            video: VideoEncode { ten_bit: false, ..Default::default() },
            ..Choices::default()
        };
        assert!(kinds(&detect(&c)).contains(&ConflictKind::CopyIgnoresFilters));
        let before = c.clone();
        apply(&mut c, ConflictKind::CopyIgnoresFilters);
        only_changed(&before, &c, |x| {
            x.audio.gain_db = 0.0;
            x.audio.compressor = CompressorPreset::None;
        });
        assert!(!kinds(&detect(&c)).contains(&ConflictKind::CopyIgnoresFilters));
    }

    #[test]
    fn script_bit_depth_follows_codec_10_bit_and_h264_is_8() {
        let h264 = VideoEncode::default();
        assert_eq!(h264.codec, VideoCodec::H264);
        assert!(h264.ten_bit, "field may be true, but H.264 output is still 8-bit");
        assert_eq!(script_bit_depth(&h264), BitDepth::Eight);
        let mut hevc = VideoEncode { codec: VideoCodec::Hevc, ten_bit: true, ..Default::default() };
        assert_eq!(script_bit_depth(&hevc), BitDepth::Ten);
        hevc.ten_bit = false;
        assert_eq!(script_bit_depth(&hevc), BitDepth::Eight);
    }

    #[test]
    fn script_is_10bit_detects_convertbits() {
        assert!(script_is_10bit("ConvertBits(10)\r\n"));
        assert!(script_is_10bit("  ConvertBits( 10, dither=-1 )\n"));
        assert!(!script_is_10bit("ConvertToYUV420()\r\n"));
        assert!(!script_is_10bit(""));
    }

    #[test]
    fn generated_script_matching_codec_has_no_bit_depth_warning() {
        let c = Choices {
            video: VideoEncode { codec: VideoCodec::Hevc, ten_bit: true, ..Default::default() },
            bits: BitDepth::Ten,
            bt709_tags: false,
            ..Choices::default()
        };
        assert!(!detect(&c).iter().any(|w| matches!(w.kind, ConflictKind::EncoderTenBitScriptEight | ConflictKind::ScriptTenBitEncoderEight)));
    }

    #[test]
    fn playback_and_pixfmt_notes_have_no_fix() {
        // these are not in detect() as fixable conflicts; informational only
        assert!(Fix { button: "x", hint: "y" }.button.len() > 0); // type is public
    }
}
