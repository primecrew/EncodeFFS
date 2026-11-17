//! Remux: stream-copy into a chosen container (MKV, MP4, MOV, WebM, TS). Never overwrites.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::p;
use crate::ffmpeg::probe::MediaInfo;
use crate::util::{ext_lower, stem};

/// Video containers this preset can write. There is no AVI option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Container {
    Mkv,
    /// Default so older "Remux MKV -> MP4" presets and settings load as Remux with MP4.
    #[default]
    Mp4,
    Mov,
    Webm,
    Ts,
}

impl Container {
    pub const ALL: [Container; 5] = [Container::Mkv, Container::Mp4, Container::Mov, Container::Webm, Container::Ts];

    pub fn ext(self) -> &'static str {
        match self {
            Container::Mkv => "mkv",
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Webm => "webm",
            Container::Ts => "ts",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Container::Mkv => "MKV",
            Container::Mp4 => "MP4",
            Container::Mov => "MOV",
            Container::Webm => "WebM",
            Container::Ts => "TS",
        }
    }

    /// "an MP4", "a WebM", "an MKV", "a MOV", "a TS".
    pub fn a_name(self) -> &'static str {
        match self {
            Container::Mkv => "an MKV",
            Container::Mp4 => "an MP4",
            Container::Mov => "a MOV",
            Container::Webm => "a WebM",
            Container::Ts => "a TS",
        }
    }
}

/// Input types the Remux list and file picker accept (not AviSynth scripts).
pub fn is_remux_input(path: &Path) -> bool {
    matches!(
        ext_lower(path).as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "m4v" | "webm" | "ts" | "mts" | "m2ts" | "flv" | "wmv" | "mpg" | "mpeg"
    )
}

/// File-dialog filters: (label, extensions).
pub const INPUT_FILTER: (&str, &[&str]) = ("Video", &["mkv", "mp4", "mov", "m4v", "webm", "ts", "mts", "m2ts", "avi", "flv", "wmv", "mpg", "mpeg"]);

#[derive(Debug, Clone, PartialEq)]
pub struct RemuxJob {
    pub source: PathBuf,
    pub dest: PathBuf,
    /// ffmpeg writes here first; renamed to `dest` only after success.
    pub temp: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemuxPlan {
    pub jobs: Vec<RemuxJob>,
    /// (source, reason)
    pub skipped: Vec<(PathBuf, String)>,
}

/// Plan the queue: one job per selected file, skipping existing outputs, same-container
/// sources, and duplicates. `exists` is injected so the logic is testable without touching the disk.
pub fn plan(
    sources: &[PathBuf],
    dest_dir: &Path,
    container: Container,
    exists: &dyn Fn(&Path) -> bool,
    mut temp_id: impl FnMut() -> String,
) -> RemuxPlan {
    let ext = container.ext();
    let mut plan = RemuxPlan::default();
    for src in sources {
        if !is_remux_input(src) {
            plan.skipped.push((src.clone(), "not a video file".into()));
            continue;
        }
        if ext_lower(src) == ext {
            plan.skipped.push((src.clone(), format!("already {}", container.a_name())));
            continue;
        }
        let dest = dest_dir.join(format!("{}.{ext}", stem(src)));
        if exists(&dest) {
            plan.skipped.push((src.clone(), format!("output already exists: {}", dest.display())));
            continue;
        }
        if plan.jobs.iter().any(|j| j.dest == dest) {
            plan.skipped.push((src.clone(), format!("another selected file already maps to {}", dest.display())));
            continue;
        }
        let temp = dest_dir.join(format!(".remux_{}.{ext}", temp_id()));
        plan.jobs.push(RemuxJob { source: src.clone(), dest, temp });
    }
    plan
}

/// `-nostdin -n -i SRC -map 0 -c copy TEMP` (`-n` = never overwrite).
/// For MKV → MP4 this stays byte-identical to 0.6.0-rc.4.
pub fn args(job: &RemuxJob) -> Vec<String> {
    vec![
        "-nostdin".into(), "-n".into(), "-i".into(), p(&job.source), "-map".into(), "0".into(), "-c".into(), "copy".into(),
        p(&job.temp),
    ]
}

/// Move `temp` to `dest` without ever replacing an existing `dest`.
///
/// `std::fs::rename` silently replaces on Windows, so we first try a hard link
/// (fails atomically if `dest` exists) and remove the temp; if the file system
/// cannot hard-link (FAT/exFAT, some network shares) we fall back to
/// check-then-rename.
pub fn publish_no_overwrite(temp: &Path, dest: &Path) -> io::Result<()> {
    let meta = std::fs::metadata(temp)?;
    if meta.len() == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "FFmpeg did not create a nonempty output"));
    }
    match std::fs::hard_link(temp, dest) {
        Ok(()) => {
            std::fs::remove_file(temp)?;
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("output appeared while processing: {}", dest.display()),
        )),
        Err(_) => {
            if dest.exists() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("output exists: {}", dest.display())));
            }
            std::fs::rename(temp, dest)
        }
    }
}

/// Queue summary in the script's wording.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemuxSummary {
    pub converted: usize,
    pub failed: usize,
    pub skipped: usize,
    pub deleted: usize,
    pub delete_failures: usize,
}
impl RemuxSummary {
    pub fn text(&self) -> String {
        format!(
            "Queue finished: {} converted, {} failed, {} skipped, {} source file(s) deleted, {} deletion failure(s).",
            self.converted, self.failed, self.skipped, self.deleted, self.delete_failures
        )
    }
}

// ----- container compatibility (FFmpeg 7.1.5 muxer codec tag lists) -----
//
// MP4: `codec_mp4_tags` in libavformat/movenc.c. `truehd` is listed there but needs
// `-strict experimental`, which this preset does not pass, so it is omitted.
// MOV: `ff_codec_movvideo_tags`, `ff_codec_movaudio_tags`, `ff_codec_movsubtitle_tags`
// in libavformat/isom_tags.c and isom.c (MODE_MOV). `truehd` is on the audio list but
// uses the same movenc writer as MP4, so it is omitted for the same `-strict` reason.
// WebM: `ff_webm_codec_tags` in libavformat/matroska.c — VP8/VP9/AV1, Vorbis/Opus, WebVTT only
// (the muxer says so explicitly).
// MPEG-TS: stream types accepted in libavformat/mpegtsenc.c.
// Matroska: `ff_mkv_codec_tags` in libavformat/matroska.c. Attachments are allowed in MKV.

const MP4_COPY_VIDEO: &[&str] = &[
    "mpeg4", "h264", "hevc", "vvc", "evc", "mpeg2video", "mpeg1video", "mjpeg", "png", "jpeg2000", "vc1",
    "dirac", "tscc2", "vp9", "av1",
];
const MP4_COPY_AUDIO: &[&str] = &[
    "aac", "alac", "mp4als", "mp3", "mp2", "ac3", "eac3", "dts", "dca", "flac", "opus", "vorbis", "qcelp",
    "evrc", "mpegh_3d_audio", "pcm_s16be", "pcm_s16le", "pcm_s24be", "pcm_s24le", "pcm_s32be", "pcm_s32le",
    "pcm_f32be", "pcm_f32le", "pcm_f64be", "pcm_f64le",
];
/// FFmpeg will store these in MP4, but everyday players often cannot play them
/// (FLAC/Opus/Vorbis; PCM uses the little-used ISO/IEC 23003-5 `ipcm`/`fpcm` tags).
const MP4_LIMITED_PLAYER_AUDIO: &[&str] = &[
    "flac", "opus", "vorbis", "pcm_s16be", "pcm_s16le", "pcm_s24be", "pcm_s24le", "pcm_s32be", "pcm_s32le",
    "pcm_f32be", "pcm_f32le", "pcm_f64be", "pcm_f64le",
];
const MP4_COPY_SUB: &[&str] = &["mov_text", "tx3g", "dvd_subtitle", "ttml"];

const MOV_COPY_VIDEO: &[&str] = &[
    "rawvideo", "r10k", "r210", "avui", "avrp", "v210", "v308", "v408", "v410", "y41p", "yuv4", "targa_y216",
    "mjpeg", "mjpegb", "svq1", "svq3", "mpeg4", "h263", "dvvideo", "vp3", "rpza", "cinepak", "8bps", "smc",
    "qtrle", "sgirle", "msrle", "qdraw", "cdtoons", "vvc", "hevc", "h264", "evc", "vp8", "vp9", "av1",
    "mpeg1video", "mpeg2video", "jpeg2000", "targa", "tiff", "gif", "png", "vc1", "cavs", "dirac", "dnxhd",
    "msmpeg4v3", "sgi", "dpx", "exr", "prores", "flic", "aic", "hap", "dxv", "magicyuv", "sheervideo",
    "pixlet", "notchlc", "media100", "cfhd", "lcevc",
];
const MOV_COPY_AUDIO: &[&str] = &[
    "aac", "ac3", "ac4", "adpcm_ima_qt", "alac", "amr_nb", "amr_wb", "dts", "dca", "eac3", "dvaudio", "gsm",
    "ilbc", "mace3", "mace6", "mp1", "mp2", "mp3", "nellymoser", "pcm_alaw", "pcm_f32be", "pcm_f32le",
    "pcm_f64be", "pcm_f64le", "pcm_mulaw", "pcm_s16be", "pcm_s16le", "pcm_s24be", "pcm_s24le", "pcm_s32be",
    "pcm_s32le", "pcm_s8", "pcm_u8", "qcelp", "qdm2", "qdmc", "speex", "evrc", "smv", "flac", "opus",
    "mpegh_3d_audio",
    // truehd is in ff_codec_movaudio_tags but needs `-strict experimental` (same movenc path as MP4).
];
const MOV_COPY_SUB: &[&str] = &["mov_text", "tx3g", "eia_608"];

const WEBM_COPY_VIDEO: &[&str] = &["vp8", "vp9", "av1"];
const WEBM_COPY_AUDIO: &[&str] = &["vorbis", "opus"];
const WEBM_COPY_SUB: &[&str] = &["webvtt"];

const TS_COPY_VIDEO: &[&str] = &["mpeg1video", "mpeg2video", "mpeg4", "h264", "hevc", "vvc", "cavs", "avs2", "avs3", "dirac", "vc1"];
const TS_COPY_AUDIO: &[&str] = &["mp2", "mp3", "aac", "aac_latm", "ac3", "eac3", "dts", "dca", "truehd", "opus", "pcm_bluray", "s302m"];
const TS_COPY_SUB: &[&str] = &["dvb_subtitle", "dvb_teletext", "arib_caption", "hdmv_pgs_subtitle", "hdmv_text_subtitle"];

const MKV_COPY_VIDEO: &[&str] = &[
    "av1", "avs2", "avs3", "dirac", "ffv1", "jpeg2000", "mjpeg", "mpeg1video", "mpeg2video", "mpeg4", "h264",
    "hevc", "msmpeg4v3", "prores", "rv10", "rv20", "rv30", "rv40", "snow", "theora", "rawvideo", "vp8", "vp9",
];
const MKV_COPY_AUDIO: &[&str] = &[
    "aac", "ac3", "alac", "atrac1", "dts", "dca", "eac3", "flac", "mlp", "mp2", "mp1", "mp3", "opus",
    "pcm_f32le", "pcm_f64le", "pcm_s16be", "pcm_s24be", "pcm_s32be", "pcm_s16le", "pcm_s24le", "pcm_s32le",
    "pcm_u8", "qdmc", "qdm2", "ra_144", "ra_288", "atrac3", "cook", "sipr", "truehd", "tta", "vorbis", "wavpack",
];
const MKV_COPY_SUB: &[&str] = &[
    "webvtt", "subrip", "srt", "text", "ass", "ssa", "dvd_subtitle", "dvb_subtitle", "hdmv_pgs_subtitle",
    "hdmv_text_subtitle", "arib_caption",
];

pub fn codec_label(codec: &str) -> String {
    match codec {
        "truehd" => "TrueHD".into(),
        "mlp" => "MLP".into(),
        "flac" => "FLAC".into(),
        "opus" => "Opus".into(),
        "vorbis" => "Vorbis".into(),
        "subrip" | "srt" => "SRT".into(),
        "ass" | "ssa" => "ASS".into(),
        "webvtt" => "WebVTT".into(),
        "hdmv_pgs_subtitle" => "PGS".into(),
        "dvd_subtitle" => "DVD".into(),
        "dvb_subtitle" => "DVB".into(),
        "eia_608" => "EIA-608".into(),
        "mov_text" | "tx3g" => "mov_text".into(),
        "h264" => "H.264".into(),
        "hevc" => "HEVC".into(),
        "av1" => "AV1".into(),
        "vp8" => "VP8".into(),
        "vp9" => "VP9".into(),
        "mpeg2video" => "MPEG-2".into(),
        "mpeg4" => "MPEG-4".into(),
        "theora" => "Theora".into(),
        "prores" => "ProRes".into(),
        c if c.starts_with("pcm_") => format!("PCM {}", c.trim_start_matches("pcm_")),
        c => c.to_string(),
    }
}

fn norm(codec: &str) -> String {
    match codec.trim().to_ascii_lowercase().as_str() {
        "dca" => "dts".into(),
        "h265" => "hevc".into(),
        "avc" => "h264".into(),
        "srt" => "subrip".into(),
        "ssa" => "ass".into(),
        "mpeg2" => "mpeg2video".into(),
        "mpeg1" => "mpeg1video".into(),
        c => c.to_string(),
    }
}

fn lists(dest: Container) -> (&'static [&'static str], &'static [&'static str], &'static [&'static str], bool, &'static [&'static str]) {
    // video, audio, sub, attachments_ok, limited_player_audio
    match dest {
        Container::Mp4 => (MP4_COPY_VIDEO, MP4_COPY_AUDIO, MP4_COPY_SUB, false, MP4_LIMITED_PLAYER_AUDIO),
        Container::Mov => (MOV_COPY_VIDEO, MOV_COPY_AUDIO, MOV_COPY_SUB, false, &[]),
        Container::Webm => (WEBM_COPY_VIDEO, WEBM_COPY_AUDIO, WEBM_COPY_SUB, false, &[]),
        Container::Ts => (TS_COPY_VIDEO, TS_COPY_AUDIO, TS_COPY_SUB, false, &[]),
        Container::Mkv => (MKV_COPY_VIDEO, MKV_COPY_AUDIO, MKV_COPY_SUB, true, &[]),
    }
}

fn in_list(c: &str, list: &[&str]) -> bool {
    let n = norm(c);
    list.contains(&n.as_str()) || list.contains(&c)
}

/// Warnings for stream-copy into MP4 (kept so existing tests and call sites stay valid).
pub fn copy_warnings(info: &MediaInfo) -> Vec<String> {
    copy_warnings_for(info, Container::Mp4)
}

/// Note shown when the output is MPEG-TS (chapters and track titles are not really kept).
pub const TS_CHAPTERS_NOTE: &str = "Chapters and track names are mostly lost in MPEG-TS.";

/// Whether the source has chapter markers or a named track (for the TS note).
pub fn has_chapters_or_titles(info: &MediaInfo) -> bool {
    info.has_chapters() || info.streams.iter().any(|s| s.title.as_deref().is_some_and(|t| !t.trim().is_empty()))
}

/// Warnings for streams that `-map 0 -c copy` into `dest` will reject, or that
/// many players will not play (MP4 only, as before). Does not change the remux command (warn, don't block).
pub fn copy_warnings_for(info: &MediaInfo, dest: Container) -> Vec<String> {
    let (vid, aud, sub, attachments_ok, limited) = lists(dest);
    let mut out = Vec::new();
    for (i, v) in info.of_type("video").enumerate() {
        if v.attached_pic {
            continue;
        }
        let c = norm(&v.codec_name);
        if !in_list(&c, vid) {
            out.push(format!(
                "Video track {} ({}) can't be copied into {}. Remux to MKV, or re-encode that video.",
                i + 1,
                codec_label(&c),
                dest.a_name()
            ));
        }
    }
    for (i, a) in info.of_type("audio").enumerate() {
        let c = norm(&a.codec_name);
        let label = format!("Track {} ({})", i + 1, codec_label(&c));
        if !in_list(&c, aud) {
            if dest == Container::Mp4 {
                // Keep the exact rc.4 wording for MP4.
                out.push(format!("{label} can't be copied into an MP4. Remux to MKV, or re-encode that audio."));
            } else {
                out.push(format!("{label} can't be copied into {}. Remux to MKV, or re-encode that audio.", dest.a_name()));
            }
        } else if dest == Container::Mp4 && in_list(&c, limited) {
            out.push(format!("{label} can be stored in MP4, but many players will not play it. Remux to MKV for wider support."));
        }
    }
    for (i, s) in info.of_type("subtitle").enumerate() {
        let c = norm(&s.codec_name);
        if !in_list(&c, sub) {
            if dest == Container::Mp4 {
                out.push(format!(
                    "Subtitle track {} ({}) can't be copied into an MP4. Remux to MKV, or convert that subtitle.",
                    i + 1,
                    codec_label(&c)
                ));
            } else {
                out.push(format!(
                    "Subtitle track {} ({}) can't be copied into {}. Remux to MKV, or convert that subtitle.",
                    i + 1,
                    codec_label(&c),
                    dest.a_name()
                ));
            }
        }
    }
    if info.has_attachments() && !attachments_ok {
        if dest == Container::Mp4 {
            out.push("Attachments (for example fonts) can't be copied into an MP4.".into());
        } else {
            out.push(format!("Attachments (for example fonts) can't be copied into {}.", dest.a_name()));
        }
    }
    if dest == Container::Ts && has_chapters_or_titles(info) {
        out.push(TS_CHAPTERS_NOTE.into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn plan_skips_existing_and_non_mkv() {
        let existing: HashSet<PathBuf> = [PathBuf::from("/out/b.mp4")].into_iter().collect();
        let mut n = 0;
        let plan = plan(
            &["/in/a.mkv".into(), "/in/b.mkv".into(), "/in/c.avi".into(), "/in2/a.mkv".into()],
            Path::new("/out"),
            Container::Mp4,
            &|p| existing.contains(p),
            || {
                n += 1;
                n.to_string()
            },
        );
        // c.avi is a video file, so it is planned (AVI is a valid remux input now).
        assert_eq!(plan.jobs.len(), 2, "{:?}", plan.jobs);
        assert_eq!(plan.jobs[0].dest, PathBuf::from("/out/a.mp4"));
        assert_eq!(plan.jobs[0].temp, PathBuf::from("/out/.remux_1.mp4"));
        assert_eq!(plan.jobs[1].dest, PathBuf::from("/out/c.mp4"));
        assert_eq!(plan.skipped.len(), 2);
        assert_eq!(
            args(&plan.jobs[0]).join(" "),
            "-nostdin -n -i /in/a.mkv -map 0 -c copy /out/.remux_1.mp4"
        );
    }

    #[test]
    fn plan_skips_files_already_in_the_chosen_container() {
        let mut n = 0;
        let plan = plan(
            &["/in/a.mp4".into(), "/in/b.mkv".into(), "/in/c.MP4".into()],
            Path::new("/out"),
            Container::Mp4,
            &|_| false,
            || {
                n += 1;
                n.to_string()
            },
        );
        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].source, PathBuf::from("/in/b.mkv"));
        assert_eq!(plan.skipped[0].1, "already an MP4");
        assert_eq!(plan.skipped[1].1, "already an MP4");
    }

    #[test]
    fn plan_webm_uses_webm_extension() {
        let mut n = 0;
        let plan = plan(&["/in/a.mkv".into()], Path::new("/out"), Container::Webm, &|_| false, || {
            n += 1;
            n.to_string()
        });
        assert_eq!(plan.jobs[0].dest, PathBuf::from("/out/a.webm"));
        assert_eq!(plan.jobs[0].temp, PathBuf::from("/out/.remux_1.webm"));
        assert_eq!(
            args(&plan.jobs[0]).join(" "),
            "-nostdin -n -i /in/a.mkv -map 0 -c copy /out/.remux_1.webm"
        );
    }

    #[test]
    fn publish_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("remux_test_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let temp = dir.join(".remux_x.mp4");
        let dest = dir.join("a.mp4");
        std::fs::write(&temp, b"data").unwrap();
        publish_no_overwrite(&temp, &dest).unwrap();
        assert!(!temp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
        // second publish must fail and keep the original
        std::fs::write(&temp, b"new").unwrap();
        assert!(publish_no_overwrite(&temp, &dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
        // empty temp is rejected
        std::fs::write(&temp, b"").unwrap();
        assert!(publish_no_overwrite(&temp, &dir.join("b.mp4")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn summary_text() {
        let s = RemuxSummary { converted: 2, failed: 1, skipped: 3, deleted: 2, delete_failures: 0 };
        assert_eq!(s.text(), "Queue finished: 2 converted, 1 failed, 3 skipped, 2 source file(s) deleted, 0 deletion failure(s).");
    }

    fn info(streams: &[(&str, &str)]) -> crate::ffmpeg::probe::MediaInfo {
        use crate::ffmpeg::probe::StreamInfo;
        crate::ffmpeg::probe::MediaInfo {
            streams: streams
                .iter()
                .enumerate()
                .map(|(i, (ty, codec))| StreamInfo {
                    index: i as u32,
                    codec_type: (*ty).into(),
                    codec_name: (*codec).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn compatible_aac_is_silent() {
        let w = copy_warnings(&info(&[("video", "h264"), ("audio", "aac")]));
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn rejected_audio_and_subs_are_named() {
        let w = copy_warnings(&info(&[
            ("video", "h264"),
            ("audio", "aac"),
            ("audio", "truehd"),
            ("audio", "mlp"),
            ("audio", "pcm_u8"),
            ("subtitle", "subrip"),
            ("subtitle", "ass"),
            ("subtitle", "hdmv_pgs_subtitle"),
            ("subtitle", "webvtt"),
            ("attachment", "ttf"),
        ]));
        assert_eq!(
            w,
            [
                "Track 2 (TrueHD) can't be copied into an MP4. Remux to MKV, or re-encode that audio.",
                "Track 3 (MLP) can't be copied into an MP4. Remux to MKV, or re-encode that audio.",
                "Track 4 (PCM u8) can't be copied into an MP4. Remux to MKV, or re-encode that audio.",
                "Subtitle track 1 (SRT) can't be copied into an MP4. Remux to MKV, or convert that subtitle.",
                "Subtitle track 2 (ASS) can't be copied into an MP4. Remux to MKV, or convert that subtitle.",
                "Subtitle track 3 (PGS) can't be copied into an MP4. Remux to MKV, or convert that subtitle.",
                "Subtitle track 4 (WebVTT) can't be copied into an MP4. Remux to MKV, or convert that subtitle.",
                "Attachments (for example fonts) can't be copied into an MP4.",
            ]
        );
    }

    #[test]
    fn flac_opus_vorbis_and_muxable_pcm_get_player_warning() {
        for (codec, label) in [
            ("flac", "FLAC"),
            ("opus", "Opus"),
            ("vorbis", "Vorbis"),
            ("pcm_s24le", "PCM s24le"),
        ] {
            let w = copy_warnings(&info(&[("audio", codec)]));
            assert_eq!(
                w,
                [format!("Track 1 ({label}) can be stored in MP4, but many players will not play it. Remux to MKV for wider support.")]
            );
        }
        let w = copy_warnings(&info(&[("audio", "ac3"), ("audio", "dts"), ("subtitle", "mov_text")]));
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn remux_args_stay_byte_identical() {
        let j = RemuxJob { source: "/in/a.mkv".into(), dest: "/out/a.mp4".into(), temp: "/out/.remux_1.mp4".into() };
        assert_eq!(args(&j).join(" "), "-nostdin -n -i /in/a.mkv -map 0 -c copy /out/.remux_1.mp4");
    }

    #[test]
    fn old_saved_container_defaults_to_mp4() {
        let c: Container = serde_json::from_value(serde_json::json!({})).unwrap_or_default();
        assert_eq!(c, Container::Mp4);
        let c: Container = serde_json::from_str("\"Mp4\"").unwrap();
        assert_eq!(c, Container::Mp4);
    }

    #[test]
    fn webm_rejects_h264_aac_and_attachments() {
        let w = copy_warnings_for(
            &info(&[("video", "h264"), ("audio", "aac"), ("subtitle", "subrip"), ("attachment", "ttf")]),
            Container::Webm,
        );
        assert_eq!(
            w,
            [
                "Video track 1 (H.264) can't be copied into a WebM. Remux to MKV, or re-encode that video.",
                "Track 1 (aac) can't be copied into a WebM. Remux to MKV, or re-encode that audio.",
                "Subtitle track 1 (SRT) can't be copied into a WebM. Remux to MKV, or convert that subtitle.",
                "Attachments (for example fonts) can't be copied into a WebM.",
            ]
        );
        let w = copy_warnings_for(&info(&[("video", "vp9"), ("audio", "opus"), ("subtitle", "webvtt")]), Container::Webm);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn mkv_accepts_common_tracks_and_attachments() {
        let w = copy_warnings_for(
            &info(&[
                ("video", "h264"),
                ("audio", "truehd"),
                ("subtitle", "ass"),
                ("subtitle", "hdmv_pgs_subtitle"),
                ("attachment", "ttf"),
            ]),
            Container::Mkv,
        );
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn ts_notes_chapters_and_rejects_flac() {
        let mut m = info(&[("video", "h264"), ("audio", "flac")]);
        m.chapters.push(crate::ffmpeg::probe::ProbeChapter { start: 0.0, end: 1.0, title: "Intro".into() });
        let w = copy_warnings_for(&m, Container::Ts);
        assert!(w.iter().any(|s| s == TS_CHAPTERS_NOTE), "{w:?}");
        assert!(w.iter().any(|s| s.contains("FLAC") && s.contains("a TS")), "{w:?}");
    }

    #[test]
    fn mp4_video_that_is_not_on_the_muxer_list_is_warned() {
        let w = copy_warnings(&info(&[("video", "theora"), ("audio", "aac")]));
        assert_eq!(
            w,
            ["Video track 1 (Theora) can't be copied into an MP4. Remux to MKV, or re-encode that video."]
        );
    }
}
