//! Mode 9: MKV -> MP4 container copy, never overwriting anything.

use std::io;
use std::path::{Path, PathBuf};

use super::p;
use crate::ffmpeg::probe::MediaInfo;
use crate::util::stem;

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

/// Plan the queue: one job per selected MKV, skipping existing outputs and duplicates.
/// `exists` is injected so the logic is testable without touching the disk.
pub fn plan(
    sources: &[PathBuf],
    dest_dir: &Path,
    exists: &dyn Fn(&Path) -> bool,
    mut temp_id: impl FnMut() -> String,
) -> RemuxPlan {
    let mut plan = RemuxPlan::default();
    for src in sources {
        if crate::util::ext_lower(src) != "mkv" {
            plan.skipped.push((src.clone(), "not an .mkv file".into()));
            continue;
        }
        let dest = dest_dir.join(format!("{}.mp4", stem(src)));
        if exists(&dest) {
            plan.skipped.push((src.clone(), format!("output already exists: {}", dest.display())));
            continue;
        }
        if plan.jobs.iter().any(|j| j.dest == dest) {
            plan.skipped.push((src.clone(), format!("another selected file already maps to {}", dest.display())));
            continue;
        }
        let temp = dest_dir.join(format!(".remux_{}.mp4", temp_id()));
        plan.jobs.push(RemuxJob { source: src.clone(), dest, temp });
    }
    plan
}

/// `-nostdin -n -i SRC -map 0 -c copy TEMP` (`-n` = never overwrite).
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
        return Err(io::Error::new(io::ErrorKind::InvalidData, "FFmpeg did not create a nonempty MP4"));
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
            "Queue finished: {} converted, {} failed, {} skipped, {} source MKV(s) deleted, {} deletion failure(s).",
            self.converted, self.failed, self.skipped, self.deleted, self.delete_failures
        )
    }
}

/// Audio codecs the FFmpeg 7.1+ MP4 muxer can stream-copy (`codec_mp4_tags`
/// in `libavformat/movenc.c`). `truehd` is listed there but needs
/// `-strict experimental`, which this preset does not pass, so it is omitted.
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

/// Subtitle codecs the MP4 muxer can stream-copy (`codec_mp4_tags`). ASS, SRT,
/// WebVTT and PGS are not on that list and fail with `-c copy`.
const MP4_COPY_SUB: &[&str] = &["mov_text", "tx3g", "dvd_subtitle", "ttml"];

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
        c if c.starts_with("pcm_") => format!("PCM {}", c.trim_start_matches("pcm_")),
        c => c.to_string(),
    }
}

fn norm(codec: &str) -> String {
    codec.trim().to_ascii_lowercase()
}

/// Warnings for streams that `-map 0 -c copy` into MP4 will reject, or that
/// many players will not play. Does not change the remux command (warn, don't block).
pub fn copy_warnings(info: &MediaInfo) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in info.of_type("audio").enumerate() {
        let c = norm(&a.codec_name);
        let label = format!("Track {} ({})", i + 1, codec_label(&c));
        if !MP4_COPY_AUDIO.contains(&c.as_str()) {
            out.push(format!("{label} can't be copied into an MP4. Remux to MKV, or re-encode that audio."));
        } else if MP4_LIMITED_PLAYER_AUDIO.contains(&c.as_str()) {
            out.push(format!("{label} can be stored in MP4, but many players will not play it. Remux to MKV for wider support."));
        }
    }
    for (i, s) in info.of_type("subtitle").enumerate() {
        let c = norm(&s.codec_name);
        if !MP4_COPY_SUB.contains(&c.as_str()) {
            out.push(format!(
                "Subtitle track {} ({}) can't be copied into an MP4. Remux to MKV, or convert that subtitle.",
                i + 1,
                codec_label(&c)
            ));
        }
    }
    if info.has_attachments() {
        out.push("Attachments (for example fonts) can't be copied into an MP4.".into());
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
            &|p| existing.contains(p),
            || {
                n += 1;
                n.to_string()
            },
        );
        assert_eq!(plan.jobs.len(), 1);
        assert_eq!(plan.jobs[0].dest, PathBuf::from("/out/a.mp4"));
        assert_eq!(plan.jobs[0].temp, PathBuf::from("/out/.remux_1.mp4"));
        assert_eq!(plan.skipped.len(), 3);
        assert_eq!(
            args(&plan.jobs[0]).join(" "),
            "-nostdin -n -i /in/a.mkv -map 0 -c copy /out/.remux_1.mp4"
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
        assert_eq!(s.text(), "Queue finished: 2 converted, 1 failed, 3 skipped, 2 source MKV(s) deleted, 0 deletion failure(s).");
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
}
