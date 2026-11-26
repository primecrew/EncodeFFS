//! GPU (hardware) video encoding (1.1): NVIDIA NVENC, Intel Quick Sync (QSV) and AMD AMF.
//!
//! One global setting (File › Settings… › Encoding hardware), off by default. When it is on and a
//! test encode proved the encoder works, the finished command's `-c:v libx264 | libx265 | libsvtav1`
//! part is swapped for the matching GPU encoder ([`rewrite`]). With the setting off nothing here
//! touches a command, so every command stays byte-identical.
//!
//! CRF → hardware quality ([`map_quality`]). The CPU presets' CRF is kept as the user's single
//! quality number; each GPU control gets it with a small offset (GPU encoders need a slightly
//! lower number than x264/x265 for a similar look) and is clamped to the encoder's range:
//!
//! | Codec (CPU CRF c)  | NVENC `-rc vbr -cq`  | QSV `-global_quality` | AMF `-rc cqp -qp_i / -qp_p (/ -qp_b)` |
//! |--------------------|----------------------|-----------------------|---------------------------------------|
//! | H.264 (x264, 0–51) | c + 1   (1–51)       | c + 1   (1–51)        | c / c + 2 / c + 4      (1–51)         |
//! | HEVC (x265, 0–51)  | c + 2   (1–51)       | c + 2   (1–51)        | c + 1 / c + 3 (no B)   (1–51)         |
//! | AV1 (SVT, 0–63)    | c       (1–63)       | c × 51/63 (1–51)      | 4c / 4c + 8 / 4c + 16  (1–255)        |
//!
//! Speed presets are fixed per vendor (the CPU preset names don't exist on GPUs): NVENC `p5`,
//! QSV `slow`, AMF `-usage transcoding -quality quality`. Pixel formats: 8-bit = `yuv420p`
//! (NVENC) / `nv12` (QSV, AMF); 10-bit HEVC/AV1 = `p010le`. Other pixel formats (4:2:2, 4:4:4)
//! stay on the CPU encoder.

use serde::{Deserialize, Serialize};

use super::codecs::VideoCodec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HwVendor {
    Nvidia,
    Intel,
    Amd,
}

impl HwVendor {
    /// Preference when more than one works (NVENC is usually the fastest and best).
    pub const ALL: [HwVendor; 3] = [HwVendor::Nvidia, HwVendor::Intel, HwVendor::Amd];
    pub fn suffix(self) -> &'static str {
        match self {
            HwVendor::Nvidia => "nvenc",
            HwVendor::Intel => "qsv",
            HwVendor::Amd => "amf",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            HwVendor::Nvidia => "NVIDIA",
            HwVendor::Intel => "Intel",
            HwVendor::Amd => "AMD",
        }
    }
}

/// `h264_nvenc`, `hevc_qsv`, `av1_amf`, …
pub fn encoder_name(codec: VideoCodec, vendor: HwVendor) -> String {
    let c = match codec {
        VideoCodec::H264 => "h264",
        VideoCodec::Hevc => "hevc",
        VideoCodec::Av1 => "av1",
    };
    format!("{c}_{}", vendor.suffix())
}

/// Which GPU encoders an `ffmpeg -hide_banner -encoders` listing names (not yet tested).
pub fn parse_encoders(encoders_out: &str) -> Vec<(VideoCodec, HwVendor)> {
    let mut v = vec![];
    for vendor in HwVendor::ALL {
        for codec in VideoCodec::ALL {
            if crate::ffmpeg::version::has_encoder(encoders_out, &encoder_name(codec, vendor)) {
                v.push((codec, vendor));
            }
        }
    }
    v
}

/// Result of the ~1 s test encode for one GPU encoder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HwTest {
    pub codec: VideoCodec,
    pub vendor: HwVendor,
    /// 8-bit test encode worked.
    pub ok: bool,
    /// 10-bit (p010le) test encode worked (HEVC / AV1 only; always false for H.264).
    pub ok_10bit: bool,
    /// Last FFmpeg error line when the 8-bit test failed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// Detection for one FFmpeg (the one in use, or the app-managed copy).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HwDetect {
    /// ffmpeg executable this was run on.
    pub program: String,
    pub tests: Vec<HwTest>,
    /// e.g. "NVIDIA GeForce RTX 3060" when it was cheap to read; None = vendor name only.
    pub gpu_name: Option<String>,
    /// Unix seconds.
    pub checked: u64,
}

impl HwDetect {
    /// The GPU encoder to use for `codec` (10-bit needs the 10-bit test to have passed).
    pub fn pick(&self, codec: VideoCodec, ten_bit: bool) -> Option<HwVendor> {
        HwVendor::ALL.into_iter().find(|&vd| {
            self.tests.iter().any(|t| t.codec == codec && t.vendor == vd && t.ok && (!ten_bit || t.ok_10bit))
        })
    }
    /// Codecs that can use the GPU (8-bit), in H.264, HEVC, AV1 order.
    pub fn working_codecs(&self) -> Vec<VideoCodec> {
        VideoCodec::ALL.into_iter().filter(|&c| self.pick(c, false).is_some()).collect()
    }
    /// Vendor of the first working encoder.
    pub fn vendor(&self) -> Option<HwVendor> {
        HwVendor::ALL.into_iter().find(|&vd| self.tests.iter().any(|t| t.vendor == vd && t.ok))
    }
    /// Encoders were listed, but no test encode worked (old driver, no card, …).
    pub fn listed_but_failed(&self) -> bool {
        !self.tests.is_empty() && self.tests.iter().all(|t| !t.ok)
    }
}

/// The global setting, stored in the settings file (`"hw"`); everything defaults to off / empty.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HwSettings {
    /// "Use my GPU" (true) / "CPU only" (false, the default).
    pub use_gpu: bool,
    /// Results per FFmpeg executable.
    pub detect: Vec<HwDetect>,
    /// The one-time "turn it on?" nudge was shown.
    pub nudge_done: bool,
}

impl HwSettings {
    pub fn for_program(&self, program: &str) -> Option<&HwDetect> {
        self.detect.iter().find(|d| d.program == program)
    }
    /// Replace (or add) the result for `d.program`.
    pub fn store(&mut self, d: HwDetect) {
        self.detect.retain(|x| x.program != d.program);
        self.detect.push(d);
    }
    /// Detection to apply to commands run with `program`, only when "Use my GPU" is on.
    pub fn active_for(&self, program: &str) -> Option<&HwDetect> {
        if self.use_gpu {
            self.for_program(program)
        } else {
            None
        }
    }
}

/// Pixel format for the GPU encoder.
pub fn pix_fmt(vendor: HwVendor, ten_bit: bool) -> &'static str {
    match (vendor, ten_bit) {
        (_, true) => "p010le",
        (HwVendor::Nvidia, false) => "yuv420p",
        (_, false) => "nv12",
    }
}

/// CRF the CPU encoder uses when the command has no `-crf` (libx264 / libx265 / SVT-AV1 defaults).
pub fn cpu_default_crf(codec: VideoCodec) -> u32 {
    match codec {
        VideoCodec::H264 => 23,
        VideoCodec::Hevc => 28,
        VideoCodec::Av1 => 35,
    }
}

/// The documented CRF → hardware quality table (see the module docs). Returns the main value
/// (NVENC `-cq`, QSV `-global_quality`, AMF `-qp_i`).
pub fn map_quality(codec: VideoCodec, vendor: HwVendor, crf: u32) -> u32 {
    let c = crf as i64;
    let (v, lo, hi) = match (codec, vendor) {
        (VideoCodec::H264, HwVendor::Nvidia | HwVendor::Intel) => (c + 1, 1, 51),
        (VideoCodec::H264, HwVendor::Amd) => (c, 1, 51),
        (VideoCodec::Hevc, HwVendor::Nvidia | HwVendor::Intel) => (c + 2, 1, 51),
        (VideoCodec::Hevc, HwVendor::Amd) => (c + 1, 1, 51),
        (VideoCodec::Av1, HwVendor::Nvidia) => (c, 1, 63),
        (VideoCodec::Av1, HwVendor::Intel) => ((c * 51 + 31) / 63, 1, 51),
        (VideoCodec::Av1, HwVendor::Amd) => (c * 4, 1, 255),
    };
    v.clamp(lo, hi) as u32
}

/// Quality + speed options that follow `-c:v <gpu encoder>`.
pub fn quality_args(codec: VideoCodec, vendor: HwVendor, crf: u32) -> Vec<String> {
    let q = map_quality(codec, vendor, crf);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    match vendor {
        HwVendor::Nvidia => s(&["-rc", "vbr", "-cq", &q.to_string(), "-b:v", "0", "-preset", "p5"]),
        HwVendor::Intel => s(&["-global_quality", &q.to_string(), "-preset", "slow"]),
        HwVendor::Amd => {
            let (step, hi) = if codec == VideoCodec::Av1 { (8, 255) } else { (2, 51) };
            let p = (q + step).min(hi).to_string();
            let mut a = s(&["-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", &q.to_string(), "-qp_p", &p]);
            if codec != VideoCodec::Hevc {
                a.extend(s(&["-qp_b", &(q + 2 * step).min(hi).to_string()]));
            }
            a
        }
    }
}

/// ~1 second test encode (lavfi test picture to the null muxer). `ffmpeg` is prepended by the caller.
pub fn test_args(codec: VideoCodec, vendor: HwVendor, ten_bit: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-v", "error", "-f", "lavfi", "-i", "testsrc2=s=1280x720:r=30:d=1", "-frames:v", "30", "-c:v"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.push(encoder_name(codec, vendor));
    a.extend(quality_args(codec, vendor, codec.default_crf(false)));
    a.extend(["-pix_fmt".to_string(), pix_fmt(vendor, ten_bit).to_string(), "-f".into(), "null".into(), "-".into()]);
    a
}

/// A command that now uses the GPU, with the original kept for the CPU fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuCommand {
    pub args: Vec<String>,
    /// e.g. "hevc_nvenc"
    pub encoder: String,
    pub vendor: HwVendor,
    pub codec: VideoCodec,
}

fn cpu_codec(name: &str) -> Option<VideoCodec> {
    match name {
        "libx264" => Some(VideoCodec::H264),
        "libx265" => Some(VideoCodec::Hevc),
        "libsvtav1" => Some(VideoCodec::Av1),
        _ => None,
    }
}

/// Swap the CPU video encoder in a finished command for the GPU one, when `det` has a working
/// encoder for that codec (and bit depth). None = leave the command exactly as it is: no CPU
/// video encoder, more than one, an unusual pixel format, or no working GPU encoder.
pub fn rewrite(args: &[String], det: &HwDetect) -> Option<GpuCommand> {
    let enc_at: Vec<usize> = (0..args.len().saturating_sub(1)).filter(|&i| args[i] == "-c:v" && cpu_codec(&args[i + 1]).is_some()).collect();
    if enc_at.len() != 1 {
        return None;
    }
    let at = enc_at[0];
    let codec = cpu_codec(&args[at + 1])?;
    let value_of = |k: &str| (0..args.len().saturating_sub(1)).find(|&i| args[i] == k).map(|i| args[i + 1].as_str());
    let ten_bit = match value_of("-pix_fmt") {
        None | Some("yuv420p") => false,
        Some("yuv420p10le") => true,
        Some(_) => return None, // 4:2:2 / 4:4:4 etc.: no GPU encoder takes these here
    };
    if codec == VideoCodec::H264 && ten_bit {
        return None;
    }
    let vendor = det.pick(codec, ten_bit)?;
    let crf = value_of("-crf").and_then(|c| c.parse().ok()).unwrap_or_else(|| cpu_default_crf(codec));
    let encoder = encoder_name(codec, vendor);
    let mut out = Vec::with_capacity(args.len() + 8);
    let mut has_pix = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if i == at {
            out.push("-c:v".into());
            out.push(encoder.clone());
            out.extend(quality_args(codec, vendor, crf));
            i += 2;
            continue;
        }
        match a {
            // replaced by quality_args, or CPU-only options
            "-crf" | "-preset" | "-tune" | "-x265-params" | "-svtav1-params" if i + 1 < args.len() => {
                i += 2;
                continue;
            }
            "-pix_fmt" if i + 1 < args.len() => {
                out.push("-pix_fmt".into());
                out.push(pix_fmt(vendor, ten_bit).into());
                has_pix = true;
                i += 2;
                continue;
            }
            _ => out.push(args[i].clone()),
        }
        i += 1;
    }
    if !has_pix {
        // insert right after the quality options
        let pos = out.iter().position(|x| *x == encoder).map(|p| p + 1 + quality_args(codec, vendor, crf).len()).unwrap_or(out.len());
        out.insert(pos, pix_fmt(vendor, ten_bit).into());
        out.insert(pos, "-pix_fmt".into());
    }
    Some(GpuCommand { args: out, encoder, vendor, codec })
}

/// What a built command becomes for `program` under the user's setting: None = run it as built
/// (GPU encoding off, not tested for this FFmpeg, test failed, or not a CPU re-encode).
pub fn apply(args: &[String], hw: &HwSettings, program: &str) -> Option<GpuCommand> {
    rewrite(args, hw.active_for(program)?)
}

/// Plain summary of a detection for the Settings section.
/// Codec list "A", "A and B", "A, B and C".
fn and_list(names: &[&str], last: &str) -> String {
    match names {
        [] => String::new(),
        [a] => a.to_string(),
        [rest @ .., z] => format!("{} {last} {z}", rest.join(", ")),
    }
}

/// One-line summary of what an FFmpeg can do, for "FFmpeg to use" and the FFmpeg menu, e.g.
/// "Has HEVC, AV1 and AC3 · GPU: NVIDIA NVENC (H.264, HEVC, AV1)" or "Has AC3, no HEVC or AV1 · GPU: none".
/// `enc` None = not checked yet; `det` None = GPU not checked yet.
pub fn support_line(enc: Option<&super::codecs::Encoders>, det: Option<&HwDetect>) -> String {
    let codecs = match enc {
        None => "Checking codecs…".to_string(),
        Some(e) => {
            let all = [("HEVC", e.libx265), ("AV1", e.libsvtav1), ("AC3", e.ac3)];
            let has: Vec<&str> = all.iter().filter(|x| x.1).map(|x| x.0).collect();
            let no: Vec<&str> = all.iter().filter(|x| !x.1).map(|x| x.0).collect();
            match (has.is_empty(), no.is_empty()) {
                (false, true) => format!("Has {}", and_list(&has, "and")),
                (false, false) => format!("Has {}, no {}", and_list(&has, "and"), and_list(&no, "or")),
                _ => format!("No {}", and_list(&no, "or")),
            }
        }
    };
    let gpu = match det {
        None => "GPU: not checked yet".to_string(),
        Some(d) => match d.vendor() {
            None => "GPU: none".to_string(),
            Some(v) => {
                let names: Vec<&str> = d.working_codecs().iter().filter(|c| d.pick(**c, false) == Some(v)).map(|c| c.short()).collect();
                format!("GPU: {} {} ({})", v.name(), v.suffix().to_uppercase(), names.join(", "))
            }
        },
    };
    format!("{codecs} · {gpu}")
}

pub fn summary(det: Option<&HwDetect>) -> String {
    let Some(d) = det else { return "Not checked yet.".into() };
    let working = d.working_codecs();
    if let Some(vendor) = d.vendor() {
        let who = d.gpu_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} GPU", vendor.name()));
        let names: Vec<&str> = working.iter().map(|c| c.short()).collect();
        let list = match names.as_slice() {
            [a] => a.to_string(),
            [a, b] => format!("{a} and {b}"),
            [a, b, c] => format!("{a}, {b} and {c}"),
            _ => names.join(", "),
        };
        return format!("{who} found: {list} can use your GPU (much faster, slightly bigger files).");
    }
    if d.listed_but_failed() {
        let v: Vec<&str> = HwVendor::ALL.into_iter().filter(|&x| d.tests.iter().any(|t| t.vendor == x)).map(|x| x.name()).collect();
        let v = v.join(" and ");
        return format!("Your FFmpeg has {v} GPU encoders, but the test encode failed (no such card, or the graphics driver is too old). Encoding stays on the CPU.");
    }
    "No GPU encoder found. Encoding uses the CPU.".into()
}

/// HEVC / AV1 that work at 8-bit but not 10-bit (10-bit encodes of those stay on the CPU).
pub fn ten_bit_note(det: &HwDetect) -> Option<String> {
    let only8: Vec<&str> = [VideoCodec::Hevc, VideoCodec::Av1]
        .into_iter()
        .filter(|&c| det.pick(c, false).is_some() && det.pick(c, true).is_none())
        .map(|c| c.short())
        .collect();
    (!only8.is_empty()).then(|| format!("10-bit {} stays on the CPU (your GPU or driver can't do it).", only8.join(" and ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn det(tests: &[(VideoCodec, HwVendor, bool, bool)]) -> HwDetect {
        HwDetect {
            program: "ffmpeg".into(),
            tests: tests.iter().map(|&(codec, vendor, ok, ok_10bit)| HwTest { codec, vendor, ok, ok_10bit, error: String::new() }).collect(),
            gpu_name: None,
            checked: 0,
        }
    }

    #[test]
    fn support_line_wording() {
        use crate::cmd::codecs::Encoders;
        let all = Encoders { libx265: true, libsvtav1: true, ac3: true };
        let nv = det(&[
            (VideoCodec::H264, HwVendor::Nvidia, true, true),
            (VideoCodec::Hevc, HwVendor::Nvidia, true, true),
            (VideoCodec::Av1, HwVendor::Nvidia, true, false),
        ]);
        assert_eq!(support_line(Some(&all), Some(&nv)), "Has HEVC, AV1 and AC3 · GPU: NVIDIA NVENC (H.264, HEVC, AV1)");
        let some = Encoders { libx265: true, libsvtav1: false, ac3: true };
        assert_eq!(support_line(Some(&some), Some(&det(&[]))), "Has HEVC and AC3, no AV1 · GPU: none");
        let one = Encoders { libx265: false, libsvtav1: false, ac3: true };
        let failed = det(&[(VideoCodec::H264, HwVendor::Intel, false, false)]);
        assert_eq!(support_line(Some(&one), Some(&failed)), "Has AC3, no HEVC or AV1 · GPU: none");
        assert_eq!(support_line(Some(&Encoders::default()), None), "No HEVC, AV1 or AC3 · GPU: not checked yet");
        assert_eq!(support_line(None, None), "Checking codecs… · GPU: not checked yet");
        let qsv = det(&[(VideoCodec::H264, HwVendor::Intel, true, false), (VideoCodec::Hevc, HwVendor::Intel, true, false)]);
        assert_eq!(support_line(Some(&all), Some(&qsv)), "Has HEVC, AV1 and AC3 · GPU: Intel QSV (H.264, HEVC)");
    }

    const ENCODERS: &str = "Encoders:
 V..... = Video
 ------
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (codec h264)
 V....D h264_amf             AMD AMF H.264 Encoder (codec h264)
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V..... h264_qsv             H.264 / AVC / MPEG-4 AVC / MPEG-4 part 10 (Intel Quick Sync Video acceleration) (codec h264)
 V....D libx265              libx265 H.265 / HEVC (codec hevc)
 V....D hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
 V....D av1_nvenc            NVIDIA NVENC av1 encoder (codec av1)
 V..... av1_qsv              AV1 (Intel Quick Sync Video acceleration) (codec av1)
 V....D h264_vaapi           H.264/AVC (VAAPI) (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)
";

    #[test]
    fn parses_gpu_encoders() {
        let v = parse_encoders(ENCODERS);
        assert_eq!(
            v,
            vec![
                (VideoCodec::H264, HwVendor::Nvidia),
                (VideoCodec::Hevc, HwVendor::Nvidia),
                (VideoCodec::Av1, HwVendor::Nvidia),
                (VideoCodec::H264, HwVendor::Intel),
                (VideoCodec::Av1, HwVendor::Intel),
                (VideoCodec::H264, HwVendor::Amd),
            ]
        );
        assert!(parse_encoders(" V....D libx264  libx264\n A....D aac AAC\n").is_empty());
        // a description that merely mentions nvenc is not an encoder line
        assert!(parse_encoders("h264_nvenc is great\n").is_empty());
    }

    #[test]
    fn crf_mapping_table() {
        use HwVendor::*;
        use VideoCodec::*;
        assert_eq!(map_quality(H264, Nvidia, 22), 23);
        assert_eq!(map_quality(H264, Intel, 22), 23);
        assert_eq!(map_quality(H264, Amd, 22), 22);
        assert_eq!(map_quality(Hevc, Nvidia, 20), 22);
        assert_eq!(map_quality(Hevc, Intel, 20), 22);
        assert_eq!(map_quality(Hevc, Amd, 20), 21);
        assert_eq!(map_quality(Av1, Nvidia, 28), 28);
        assert_eq!(map_quality(Av1, Intel, 28), 23);
        assert_eq!(map_quality(Av1, Amd, 28), 112);
        // clamped to each encoder's range
        assert_eq!(map_quality(H264, Nvidia, 51), 51);
        assert_eq!(map_quality(H264, Amd, 0), 1);
        assert_eq!(map_quality(Av1, Nvidia, 63), 63);
        assert_eq!(map_quality(Av1, Intel, 63), 51);
        assert_eq!(map_quality(Av1, Amd, 63), 252);
        assert_eq!(map_quality(Av1, Amd, 0), 1);
        // AMF P/B steps stay in range
        assert_eq!(quality_args(Av1, Amd, 63), s(&["-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "252", "-qp_p", "255", "-qp_b", "255"]));
        assert_eq!(quality_args(Hevc, Amd, 20), s(&["-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "21", "-qp_p", "23"]));
        assert_eq!(quality_args(H264, Nvidia, 22), s(&["-rc", "vbr", "-cq", "23", "-b:v", "0", "-preset", "p5"]));
        assert_eq!(quality_args(H264, Intel, 22), s(&["-global_quality", "23", "-preset", "slow"]));
    }

    #[test]
    fn h264_reencode_to_nvenc() {
        let d = det(&[(VideoCodec::H264, HwVendor::Nvidia, true, false)]);
        let a = s(&["-y", "-i", "in.mkv", "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p", "-vf", "bwdif", "-tune", "film", "-c:a", "aac", "-b:a", "192k", "out.mkv"]);
        let g = rewrite(&a, &d).unwrap();
        assert_eq!(g.encoder, "h264_nvenc");
        assert_eq!(
            g.args,
            s(&["-y", "-i", "in.mkv", "-c:v", "h264_nvenc", "-rc", "vbr", "-cq", "23", "-b:v", "0", "-preset", "p5", "-pix_fmt", "yuv420p", "-vf", "bwdif", "-c:a", "aac", "-b:a", "192k", "out.mkv"])
        );
    }

    #[test]
    fn hevc_10bit_uses_p010_and_keeps_the_mp4_tag() {
        let d = det(&[(VideoCodec::Hevc, HwVendor::Nvidia, true, true)]);
        let a = s(&["-i", "in.mkv", "-c:v", "libx265", "-crf", "20", "-preset", "slow", "-pix_fmt", "yuv420p10le", "-x265-params", "log-level=error", "-tag:v", "hvc1", "out.mp4"]);
        let g = rewrite(&a, &d).unwrap();
        assert_eq!(g.args, s(&["-i", "in.mkv", "-c:v", "hevc_nvenc", "-rc", "vbr", "-cq", "22", "-b:v", "0", "-preset", "p5", "-pix_fmt", "p010le", "-tag:v", "hvc1", "out.mp4"]));
        // 10-bit test failed: the 10-bit command stays on the CPU, 8-bit uses the GPU
        let d8 = det(&[(VideoCodec::Hevc, HwVendor::Nvidia, true, false)]);
        assert_eq!(rewrite(&a, &d8), None);
        let a8: Vec<String> = a.iter().map(|x| if x == "yuv420p10le" { "yuv420p".into() } else { x.clone() }).collect();
        assert_eq!(rewrite(&a8, &d8).unwrap().encoder, "hevc_nvenc");
    }

    #[test]
    fn av1_qsv_and_amf() {
        let a = s(&["-i", "in.mkv", "-c:v", "libsvtav1", "-crf", "28", "-preset", "5", "-pix_fmt", "yuv420p10le", "-svtav1-params", "film-grain=8:film-grain-denoise=0", "out.mkv"]);
        let qsv = det(&[(VideoCodec::Av1, HwVendor::Intel, true, true)]);
        assert_eq!(rewrite(&a, &qsv).unwrap().args, s(&["-i", "in.mkv", "-c:v", "av1_qsv", "-global_quality", "23", "-preset", "slow", "-pix_fmt", "p010le", "out.mkv"]));
        let amf = det(&[(VideoCodec::Av1, HwVendor::Amd, true, false)]);
        let a8: Vec<String> = a.iter().map(|x| if x == "yuv420p10le" { "yuv420p".into() } else { x.clone() }).collect();
        assert_eq!(
            rewrite(&a8, &amf).unwrap().args,
            s(&["-i", "in.mkv", "-c:v", "av1_amf", "-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", "112", "-qp_p", "120", "-qp_b", "128", "-pix_fmt", "nv12", "out.mkv"])
        );
    }

    #[test]
    fn x264_defaults_without_crf_or_pix_fmt() {
        // MP4 from still leaves -crf / -preset at libx264's defaults
        let d = det(&[(VideoCodec::H264, HwVendor::Intel, true, false)]);
        let a = s(&["-y", "-loop", "1", "-i", "c.png", "-i", "s.flac", "-c:v", "libx264", "-tune", "stillimage", "-c:a", "aac", "out.mp4"]);
        assert_eq!(
            rewrite(&a, &d).unwrap().args,
            s(&["-y", "-loop", "1", "-i", "c.png", "-i", "s.flac", "-c:v", "h264_qsv", "-global_quality", "24", "-preset", "slow", "-pix_fmt", "nv12", "-c:a", "aac", "out.mp4"])
        );
    }

    #[test]
    fn untouched_commands() {
        let d = det(&[(VideoCodec::H264, HwVendor::Nvidia, true, false), (VideoCodec::Hevc, HwVendor::Nvidia, true, true)]);
        // copy / remux, no working GPU for the codec, unusual pixel format, two encoders
        assert_eq!(rewrite(&s(&["-i", "a.mkv", "-c:v", "copy", "-c:a", "copy", "b.mp4"]), &d), None);
        assert_eq!(rewrite(&s(&["-i", "a.mkv", "-c:v", "libsvtav1", "-crf", "28", "b.mkv"]), &d), None);
        assert_eq!(rewrite(&s(&["-i", "a.mkv", "-c:v", "libx265", "-pix_fmt", "yuv422p10le", "b.mkv"]), &d), None);
        assert_eq!(rewrite(&s(&["-i", "a", "-c:v", "libx264", "-c:v", "libx264", "b"]), &d), None);
        // failed tests never pick an encoder
        let failed = det(&[(VideoCodec::H264, HwVendor::Nvidia, false, false)]);
        assert_eq!(rewrite(&s(&["-i", "a", "-c:v", "libx264", "-crf", "22", "b.mp4"]), &failed), None);
        assert!(failed.listed_but_failed());
        // setting off: nothing is active
        let mut hw = HwSettings::default();
        hw.store(d.clone());
        assert!(hw.active_for("ffmpeg").is_none());
        hw.use_gpu = true;
        assert!(hw.active_for("ffmpeg").is_some());
        assert!(hw.active_for("other").is_none());
    }

    #[test]
    fn picks_nvidia_first_and_respects_10bit() {
        let d = det(&[
            (VideoCodec::Hevc, HwVendor::Intel, true, true),
            (VideoCodec::Hevc, HwVendor::Nvidia, true, false),
        ]);
        assert_eq!(d.pick(VideoCodec::Hevc, false), Some(HwVendor::Nvidia));
        assert_eq!(d.pick(VideoCodec::Hevc, true), Some(HwVendor::Intel));
        assert_eq!(d.pick(VideoCodec::H264, false), None);
    }

    #[test]
    fn summaries() {
        assert_eq!(summary(None), "Not checked yet.");
        let all = det(&[
            (VideoCodec::H264, HwVendor::Nvidia, true, false),
            (VideoCodec::Hevc, HwVendor::Nvidia, true, true),
            (VideoCodec::Av1, HwVendor::Nvidia, true, false),
        ]);
        assert_eq!(summary(Some(&all)), "NVIDIA GPU found: H.264, HEVC and AV1 can use your GPU (much faster, slightly bigger files).");
        assert_eq!(ten_bit_note(&all).unwrap(), "10-bit AV1 stays on the CPU (your GPU or driver can't do it).");
        let mut named = all.clone();
        named.gpu_name = Some("NVIDIA GeForce RTX 3060".into());
        assert!(summary(Some(&named)).starts_with("NVIDIA GeForce RTX 3060 found: "));
        let failed = det(&[(VideoCodec::H264, HwVendor::Amd, false, false)]);
        assert!(summary(Some(&failed)).contains("test encode failed"));
        assert_eq!(summary(Some(&det(&[]))), "No GPU encoder found. Encoding uses the CPU.");
    }

    #[test]
    fn test_encode_args() {
        let a = test_args(VideoCodec::Hevc, HwVendor::Nvidia, true);
        let j = a.join(" ");
        assert!(j.starts_with("-hide_banner -nostdin -v error -f lavfi -i testsrc2=s=1280x720:r=30:d=1 -frames:v 30 -c:v hevc_nvenc "), "{j}");
        assert!(j.ends_with("-pix_fmt p010le -f null -"), "{j}");
        assert!(test_args(VideoCodec::H264, HwVendor::Intel, false).join(" ").contains("-pix_fmt nv12"));
    }

    #[test]
    fn old_settings_files_default_to_off() {
        let h: HwSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(h, HwSettings::default());
        assert!(!h.use_gpu);
    }
}
