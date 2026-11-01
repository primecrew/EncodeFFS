//! "Custom" preset type: a pipeline composed from independent actions.
//!
//! Inputs, in order: `0` main file (`[input args] [-ss] -i`), then optionally the
//! extra audio file (replace / add track), the subtitle file (import) and the
//! FFMETADATA chapter file. Trim start is applied to the main, audio and subtitle
//! inputs alike (they are assumed to be aligned with the main file); trim end is `-t`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::common::*;
use super::reencode::{is_mp4_like, StreamRef};
use super::chapters::{self, ChapterMode, ManualChapter};
use super::{p, ArgVec, BuiltCommand};
use crate::ffmpeg::probe::MediaInfo;
use crate::util::{ext_lower, fmt_num};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VideoAction {
    Copy,
    #[default]
    X264,
    Drop,
}
impl VideoAction {
    pub const ALL: [VideoAction; 3] = [VideoAction::Copy, VideoAction::X264, VideoAction::Drop];
    pub fn label(self) -> &'static str {
        match self {
            VideoAction::Copy => "Copy (no re-encode)",
            VideoAction::X264 => "Re-encode (libx264)",
            VideoAction::Drop => "Drop video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioAction {
    Copy,
    #[default]
    Aac,
    Drop,
    Replace,
    Mix,
    AddTrack,
}
impl AudioAction {
    pub const ALL: [AudioAction; 6] = [AudioAction::Copy, AudioAction::Aac, AudioAction::Drop, AudioAction::Replace, AudioAction::Mix, AudioAction::AddTrack];
    pub fn label(self) -> &'static str {
        match self {
            AudioAction::Copy => "Copy all tracks",
            AudioAction::Aac => "Re-encode all tracks to AAC",
            AudioAction::Drop => "Drop audio",
            AudioAction::Replace => "Replace with audio file (AAC)",
            AudioAction::Mix => "Mix all tracks into one (AAC)",
            AudioAction::AddTrack => "Copy tracks + add audio file as new track",
        }
    }
    pub fn needs_file(self) -> bool {
        matches!(self, AudioAction::Replace | AudioAction::AddTrack)
    }
    /// Whether gain/compressor/bitrate apply.
    pub fn encodes(self) -> bool {
        !matches!(self, AudioAction::Copy | AudioAction::Drop)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubAction {
    #[default]
    Copy,
    Convert,
    Drop,
    Import,
}
impl SubAction {
    pub const ALL: [SubAction; 4] = [SubAction::Copy, SubAction::Convert, SubAction::Drop, SubAction::Import];
    pub fn label(self) -> &'static str {
        match self {
            SubAction::Copy => "Copy (MP4: text -> mov_text)",
            SubAction::Convert => "Convert text subs (MP4: mov_text, others: SRT)",
            SubAction::Drop => "Drop subtitles",
            SubAction::Import => "Import from another file",
        }
    }
}

/// Containers offered for custom presets.
pub const CONTAINERS: &[&str] = &["mp4", "mkv", "mov", "m4v", "m4a", "mka"];

pub fn is_audio_only_container(ext: &str) -> bool {
    matches!(ext, "m4a" | "mka")
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomParams {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Input duration (needed for trim end and chapters).
    pub duration_s: Option<f64>,
    pub trim_start: f64,
    pub trim_end: f64,
    pub video: VideoAction,
    pub video_encode: VideoEncode,
    pub resize: Option<(u32, u32)>,
    /// Caller passes `interlaced && wanted`.
    pub deinterlace: bool,
    pub range_convert: bool,
    pub audio: AudioAction,
    pub audio_encode: AudioEncode,
    pub audio_file: Option<PathBuf>,
    /// Title for an added / mixed track (empty = none).
    pub track_title: String,
    pub subs: SubAction,
    pub sub_file: Option<PathBuf>,
    pub keep_existing_subs: bool,
    pub chapters: Option<(PathBuf, String)>,
    pub drop_chapters: bool,
    pub faststart: bool,
    /// Extra options placed before the main `-i`.
    pub input_args: Vec<String>,
    /// Extra options placed just before the output file.
    pub output_args: Vec<String>,
    // facts from ffprobe
    pub has_video: bool,
    pub audio_count: usize,
    pub subtitle_streams: Vec<StreamRef>,
    pub has_attachments: bool,
}

impl CustomParams {
    pub fn output_duration(&self) -> Option<f64> {
        self.duration_s.map(|d| d - self.trim_start - self.trim_end)
    }
}

/// Split a free-form option string into arguments. Whitespace separates;
/// "double" or 'single' quotes group. Backslashes are literal (Windows paths).
pub fn split_args(s: &str) -> Result<Vec<String>, String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut have = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                have = true;
            }
            None if c.is_whitespace() => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            None => {
                cur.push(c);
                have = true;
            }
        }
    }
    if quote.is_some() {
        return Err(format!("Unclosed quote in extra arguments: {s}"));
    }
    if have {
        out.push(cur);
    }
    Ok(out)
}

/// `[0:a:0][0:a:1]…amix=inputs=N:normalize=1[,gain/compressor][aout]`
pub fn mix_graph(n: usize, chain: Option<&str>) -> String {
    let mut s: String = (0..n).map(|i| format!("[0:a:{i}]")).collect();
    s.push_str(&format!("amix=inputs={n}:normalize=1"));
    if let Some(c) = chain {
        s.push(',');
        s.push_str(c);
    }
    s.push_str("[aout]");
    s
}

pub fn build(pr: &CustomParams) -> Result<BuiltCommand, String> {
    let mut warnings = vec![];
    let mut side_files = vec![];
    if pr.trim_start < 0.0 || pr.trim_end < 0.0 {
        return Err("Negative trim values".into());
    }
    if let Some(d) = pr.duration_s {
        if d <= 0.0 {
            return Err(format!("Invalid duration: {d} seconds"));
        }
        if pr.trim_start + pr.trim_end >= d {
            return Err(format!("Trim duration ({}) exceeds the input duration ({})", fmt_num(pr.trim_start + pr.trim_end), fmt_num(d)));
        }
    } else if pr.trim_end > 0.0 || pr.chapters.is_some() {
        return Err("The input duration is unknown, so trim end / chapters can't be used.".into());
    }
    let ext = ext_lower(&pr.output);
    let mp4 = is_mp4_like(&pr.output) || ext == "m4a";
    let video_out = pr.video != VideoAction::Drop && pr.has_video;
    let audio_out = pr.audio != AudioAction::Drop && (pr.audio_count > 0 || pr.audio.needs_file());
    if pr.video != VideoAction::Drop && !pr.has_video {
        warnings.push("The input has no video stream; video action ignored.".into());
    }
    if is_audio_only_container(&ext) && video_out {
        return Err(format!(".{ext} holds audio only: set Video to \"Drop video\" or pick another container."));
    }
    if !video_out && !audio_out {
        return Err("Nothing to output: both video and audio are dropped (or the input has neither).".into());
    }
    if pr.audio.encodes() {
        pr.audio_encode.validate()?;
    }
    if pr.audio == AudioAction::Mix && pr.audio_count < 2 {
        return Err(format!("Mixing requires at least two audio tracks. Found: {}", pr.audio_count));
    }
    if matches!(pr.audio, AudioAction::Copy | AudioAction::Aac) && pr.audio_count == 0 {
        warnings.push("The input has no audio stream.".into());
    }

    let trimmed = pr.trim_start > 0.0 || pr.trim_end > 0.0;
    let ss = |a: &mut Vec<String>| {
        if pr.trim_start > 0.0 {
            a.push_all(["-ss".to_string(), fmt_num(pr.trim_start)]);
        }
    };

    // ---- inputs
    let mut a: Vec<String> = vec!["-y".into()];
    a.extend(pr.input_args.iter().cloned());
    ss(&mut a);
    a.push_all(["-i".to_string(), p(&pr.input)]);
    let mut next = 1;
    let audio_idx = if pr.audio.needs_file() {
        let f = pr.audio_file.as_ref().filter(|f| !f.as_os_str().is_empty()).ok_or("Choose the audio file for this audio action.")?;
        ss(&mut a);
        a.push_all(["-i".to_string(), p(f)]);
        next += 1;
        Some(next - 1)
    } else {
        None
    };
    let sub_idx = if pr.subs == SubAction::Import {
        let f = pr.sub_file.as_ref().filter(|f| !f.as_os_str().is_empty()).ok_or("Choose the subtitle file to import.")?;
        ss(&mut a);
        a.push_all(["-i".to_string(), p(f)]);
        next += 1;
        Some(next - 1)
    } else {
        None
    };
    let ch_idx = if let Some((path, text)) = &pr.chapters {
        a.push_all(["-f", "ffmetadata", "-i"]);
        a.push(p(path));
        side_files.push((path.clone(), text.clone()));
        next += 1;
        Some(next - 1)
    } else {
        None
    };
    if trimmed {
        if let Some(od) = pr.output_duration() {
            a.push_all(["-t".to_string(), fmt_num(od)]);
        }
    }

    // ---- video ("V" = real video, not cover art)
    if video_out {
        a.push_all(["-map", "0:V?"]);
        match pr.video {
            VideoAction::X264 => {
                let filter = join_filters(&[
                    pr.deinterlace.then(|| DEINTERLACE_FILTER.to_string()),
                    pr.resize.map(|(w, h)| format!("scale={w}:{h}")),
                    pr.range_convert.then(|| RANGE_FILTER.to_string()),
                ]);
                a.extend(x264_args(&pr.video_encode, filter.as_deref(), pr.range_convert));
            }
            _ => {
                a.push_all(["-c:v", "copy"]);
                if pr.deinterlace || pr.resize.is_some() || pr.range_convert {
                    warnings.push("Video is copied: deinterlace / resize / range conversion are ignored.".into());
                }
                if pr.trim_start > 0.0 {
                    warnings.push("Trimming with video copy cuts on keyframes, so the start may be slightly earlier than requested.".into());
                }
            }
        }
    }

    // ---- audio
    let chain = pr.audio_encode.filter_chain();
    let title_args = |a: &mut Vec<String>, n: usize| {
        let t = pr.track_title.trim();
        if !t.is_empty() {
            a.push_all([format!("-metadata:s:a:{n}"), format!("title={t}")]);
            if mp4 {
                a.push_all([format!("-metadata:s:a:{n}"), format!("handler_name={t}")]);
            }
        }
    };
    match pr.audio {
        AudioAction::Drop => {}
        AudioAction::Copy => a.push_all(["-map", "0:a?", "-c:a", "copy"]),
        AudioAction::Aac => {
            a.push_all(["-map", "0:a?"]);
            a.extend(pr.audio_encode.aac_args());
            if let Some(c) = &chain {
                a.push_all(["-filter:a".to_string(), c.clone()]);
            }
        }
        AudioAction::Replace => {
            a.push_all(["-map".to_string(), format!("{}:a:0", audio_idx.unwrap())]);
            a.extend(pr.audio_encode.aac_args());
            if let Some(c) = &chain {
                a.push_all(["-filter:a".to_string(), c.clone()]);
            }
            title_args(&mut a, 0);
        }
        AudioAction::Mix => {
            a.push_all(["-filter_complex".to_string(), mix_graph(pr.audio_count, chain.as_deref())]);
            a.push_all(["-map", "[aout]"]);
            a.extend(pr.audio_encode.aac_args());
            title_args(&mut a, 0);
        }
        AudioAction::AddTrack => {
            let n = pr.audio_count;
            a.push_all(["-map".to_string(), "0:a?".to_string(), "-map".to_string(), format!("{}:a:0", audio_idx.unwrap())]);
            a.push_all(["-c:a".to_string(), "copy".to_string(), format!("-c:a:{n}"), "aac".to_string(), format!("-b:a:{n}"), format!("{}k", pr.audio_encode.bitrate_k)]);
            if let Some(c) = &chain {
                a.push_all([format!("-filter:a:{n}"), c.clone()]);
            }
            title_args(&mut a, n);
        }
    }

    // ---- subtitles
    let keep_orig = match pr.subs {
        SubAction::Copy | SubAction::Convert => true,
        SubAction::Drop => false,
        SubAction::Import => pr.keep_existing_subs,
    };
    let mut kept: Vec<&StreamRef> = vec![];
    if keep_orig {
        for s in &pr.subtitle_streams {
            if mp4 && !mp4_sub_compatible(&s.codec) {
                warnings.push(format!("Subtitle stream {} ({}) cannot be stored in MP4 and is skipped.", s.index, s.codec));
            } else {
                kept.push(s);
            }
        }
        for s in &kept {
            a.push_all(["-map".to_string(), format!("0:{}", s.index)]);
        }
    }
    if let Some(i) = sub_idx {
        a.push_all(["-map".to_string(), format!("{i}:s")]);
    }
    if !kept.is_empty() || sub_idx.is_some() {
        let codec = if mp4 {
            "mov_text"
        } else if pr.subs == SubAction::Convert {
            "srt"
        } else {
            "copy"
        };
        a.push_all(["-c:s", codec]);
        if pr.subs == SubAction::Convert && !mp4 {
            for (k, s) in kept.iter().enumerate() {
                if !mp4_sub_compatible(&s.codec) {
                    a.push_all([format!("-c:s:{k}"), "copy".to_string()]);
                    warnings.push(format!("Subtitle stream {} ({}) is a bitmap format and can't be converted to text; copied as is.", s.index, s.codec));
                }
            }
        }
    }

    // ---- attachments (fonts) only survive in Matroska
    if pr.has_attachments {
        if mp4 {
            warnings.push("Attachments (e.g. MKV fonts) cannot be stored in MP4 and are skipped.".into());
        } else {
            a.push_all(["-map", "0:t?", "-c:t", "copy"]);
        }
    }

    // ---- chapters
    if let Some(i) = ch_idx {
        a.push_all(["-map_chapters".to_string(), i.to_string()]);
    } else if pr.drop_chapters {
        a.push_all(["-map_chapters", "-1"]);
    }
    if pr.faststart && mp4 {
        a.extend(faststart_args());
    }
    a.extend(pr.output_args.iter().cloned());
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, side_files, warnings })
}

/// The saved option set of a custom preset (everything except the files of one run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomOptions {
    /// Process every video in a folder instead of one file.
    pub batch: bool,
    pub container: String,
    pub suffix: String,
    pub trim_start: f64,
    pub trim_end: f64,
    pub video: VideoAction,
    pub video_encode: VideoEncode,
    pub resize: bool,
    pub resize_height: u32,
    pub deinterlace: bool,
    pub range_convert: bool,
    pub audio: AudioAction,
    pub audio_encode: AudioEncode,
    pub track_title: String,
    pub subs: SubAction,
    pub keep_existing_subs: bool,
    pub chapter_mode: ChapterMode,
    pub chapter_interval_min: f64,
    pub manual: Vec<ManualChapter>,
    pub drop_chapters: bool,
    pub faststart: bool,
    pub input_args: String,
    pub output_args: String,
}

impl Default for CustomOptions {
    fn default() -> Self {
        CustomOptions {
            batch: false,
            container: "mp4".into(),
            suffix: "_custom".into(),
            trim_start: 0.0,
            trim_end: 0.0,
            video: VideoAction::X264,
            video_encode: VideoEncode::default(),
            resize: false,
            resize_height: 720,
            deinterlace: true,
            range_convert: false,
            audio: AudioAction::Aac,
            audio_encode: AudioEncode::default(),
            track_title: String::new(),
            subs: SubAction::Copy,
            keep_existing_subs: true,
            chapter_mode: ChapterMode::Off,
            chapter_interval_min: 5.0,
            manual: vec![],
            drop_chapters: false,
            faststart: true,
            input_args: String::new(),
            output_args: String::new(),
        }
    }
}

impl CustomOptions {
    /// `<stem><suffix>.<container>`
    pub fn output_name(&self, input: &Path) -> String {
        format!("{}{}.{}", crate::util::stem(input), self.suffix, self.container.trim_start_matches('.'))
    }

    /// Turn the options plus one input's probe facts into builder parameters.
    pub fn params(&self, input: &Path, output: &Path, info: &MediaInfo, audio_file: Option<PathBuf>, sub_file: Option<PathBuf>) -> Result<CustomParams, String> {
        let video = info.video();
        let duration_s = info.duration().or_else(|| info.any_duration());
        let resize = if self.resize && self.video == VideoAction::X264 {
            match video.and_then(|v| Some((v.width?, v.height?))) {
                Some((w, h)) => Some(resize_dims(w, h, self.resize_height)?),
                None => return Err("Resize needs the input's video dimensions (ffprobe found none).".into()),
            }
        } else {
            None
        };
        let interlaced = video.map(|v| is_interlaced(v.field_order.as_deref())).unwrap_or(false);
        let chapters = if self.chapter_mode != ChapterMode::Off {
            let d = duration_s.ok_or("The input duration is unknown, so chapters can't be generated.")?;
            let out_dur = d - self.trim_start - self.trim_end;
            if out_dur <= 0.0 {
                return Err("Trim duration exceeds the input duration".into());
            }
            let list = chapters::build_chapters(out_dur, self.chapter_mode, self.chapter_interval_min * 60.0, &self.manual)?;
            (!list.is_empty()).then(|| (std::env::temp_dir().join(format!("ffenc_chapters_{}.txt", crate::util::unique_id())), chapters::ffmetadata(&list)))
        } else {
            None
        };
        Ok(CustomParams {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            duration_s,
            trim_start: self.trim_start,
            trim_end: self.trim_end,
            video: self.video,
            video_encode: self.video_encode.clone(),
            resize,
            deinterlace: self.deinterlace && interlaced && self.video == VideoAction::X264,
            range_convert: self.range_convert && self.video == VideoAction::X264,
            audio: self.audio,
            audio_encode: self.audio_encode.clone(),
            audio_file,
            track_title: self.track_title.clone(),
            subs: self.subs,
            sub_file,
            keep_existing_subs: self.keep_existing_subs,
            chapters,
            drop_chapters: self.drop_chapters && self.chapter_mode == ChapterMode::Off,
            faststart: self.faststart,
            input_args: split_args(&self.input_args)?,
            output_args: split_args(&self.output_args)?,
            has_video: video.is_some(),
            audio_count: info.audio_count(),
            subtitle_streams: info.subtitle_refs(),
            has_attachments: info.has_attachments(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> CustomParams {
        CustomParams {
            input: "/in/a b.mkv".into(),
            output: "/out/a b_custom.mkv".into(),
            duration_s: Some(100.0),
            has_video: true,
            audio_count: 2,
            ..Default::default()
        }
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn default_reencode_all() {
        let c = build(&base()).unwrap();
        assert_eq!(
            c.args,
            s(&["-y", "-i", "/in/a b.mkv", "-map", "0:V?", "-c:v", "libx264", "-crf", "22", "-preset", "slow", "-pix_fmt", "yuv420p",
                "-map", "0:a?", "-c:a", "aac", "-b:a", "192k", "/out/a b_custom.mkv"])
        );
        assert!(c.warnings.is_empty());
    }

    #[test]
    fn copy_everything_with_trim_and_extra_args() {
        let mut pr = base();
        pr.video = VideoAction::Copy;
        pr.audio = AudioAction::Copy;
        pr.trim_start = 10.0;
        pr.trim_end = 5.5;
        pr.input_args = s(&["-hwaccel", "auto"]);
        pr.output_args = s(&["-metadata", "comment=hi there"]);
        pr.subtitle_streams = vec![StreamRef { index: 3, codec: "subrip".into() }];
        pr.has_attachments = true;
        pr.drop_chapters = true;
        let c = build(&pr).unwrap();
        assert_eq!(
            c.args,
            s(&["-y", "-hwaccel", "auto", "-ss", "10", "-i", "/in/a b.mkv", "-t", "84.5", "-map", "0:V?", "-c:v", "copy",
                "-map", "0:a?", "-c:a", "copy", "-map", "0:3", "-c:s", "copy", "-map", "0:t?", "-c:t", "copy",
                "-map_chapters", "-1", "-metadata", "comment=hi there", "/out/a b_custom.mkv"])
        );
        assert_eq!(c.warnings.len(), 1, "keyframe warning");
    }

    #[test]
    fn x264_filters_and_mp4_subs() {
        let mut pr = base();
        pr.output = "/out/x.mp4".into();
        pr.deinterlace = true;
        pr.resize = Some((1280, 720));
        pr.range_convert = true;
        pr.faststart = true;
        pr.subtitle_streams = vec![StreamRef { index: 3, codec: "subrip".into() }, StreamRef { index: 4, codec: "hdmv_pgs_subtitle".into() }];
        pr.has_attachments = true;
        let c = build(&pr).unwrap();
        let j = c.args.join(" ");
        assert!(j.contains(&format!("-vf {DEINTERLACE_FILTER},scale=1280:720,{RANGE_FILTER} -color_range tv")), "{j}");
        assert!(j.contains("-map 0:3 -c:s mov_text"), "{j}");
        assert!(!j.contains("0:4"));
        assert!(!j.contains("0:t?"));
        assert!(j.ends_with("-movflags +faststart /out/x.mp4"));
        assert_eq!(c.warnings.len(), 2);
    }

    #[test]
    fn replace_add_mix() {
        let mut pr = base();
        pr.video = VideoAction::Copy;
        pr.audio = AudioAction::Replace;
        assert!(build(&pr).is_err(), "needs file");
        pr.audio_file = Some("/a/voice.wav".into());
        pr.audio_encode.gain_db = 3.0;
        pr.trim_start = 2.0;
        let j = build(&pr).unwrap().args.join(" ");
        assert!(j.starts_with("-y -ss 2 -i /in/a b.mkv -ss 2 -i /a/voice.wav -t 98 -map 0:V? -c:v copy -map 1:a:0 -c:a aac -b:a 192k -filter:a volume=3dB"), "{j}");

        pr.audio = AudioAction::AddTrack;
        pr.track_title = "Commentary".into();
        pr.output = "/o/x.mp4".into();
        let j = build(&pr).unwrap().args.join(" ");
        assert!(j.contains("-map 0:a? -map 1:a:0 -c:a copy -c:a:2 aac -b:a:2 192k -filter:a:2 volume=3dB -metadata:s:a:2 title=Commentary -metadata:s:a:2 handler_name=Commentary"), "{j}");

        pr.audio = AudioAction::Mix;
        pr.output = "/o/x.mkv".into();
        let c = build(&pr).unwrap();
        let j = c.args.join(" ");
        assert!(!j.contains("voice.wav"));
        assert!(j.contains("-filter_complex [0:a:0][0:a:1]amix=inputs=2:normalize=1,volume=3dB[aout] -map [aout] -c:a aac"), "{j}");
        pr.audio_count = 1;
        assert!(build(&pr).is_err());
    }

    #[test]
    fn subs_import_convert_drop() {
        let mut pr = base();
        pr.subtitle_streams = vec![StreamRef { index: 3, codec: "ass".into() }, StreamRef { index: 4, codec: "dvd_subtitle".into() }];
        pr.subs = SubAction::Convert;
        let c = build(&pr).unwrap();
        let j = c.args.join(" ");
        assert!(j.contains("-map 0:3 -map 0:4 -c:s srt -c:s:1 copy"), "{j}");
        pr.subs = SubAction::Drop;
        assert!(!build(&pr).unwrap().args.join(" ").contains("-c:s"));
        pr.subs = SubAction::Import;
        assert!(build(&pr).is_err());
        pr.sub_file = Some("/s/en.srt".into());
        let j = build(&pr).unwrap().args.join(" ");
        assert!(j.contains("-i /s/en.srt") && j.contains("-map 1:s -c:s copy") && !j.contains("0:3"), "{j}");
        pr.keep_existing_subs = true;
        let j = build(&pr).unwrap().args.join(" ");
        assert!(j.contains("-map 0:3 -map 0:4 -map 1:s -c:s copy"), "{j}");
    }

    #[test]
    fn chapters_and_errors() {
        let mut pr = base();
        pr.audio_file = Some("/a/v.wav".into());
        pr.audio = AudioAction::AddTrack;
        pr.chapters = Some(("/tmp/ch.txt".into(), ";FFMETADATA1\n".into()));
        let c = build(&pr).unwrap();
        let j = c.args.join(" ");
        assert!(j.contains("-f ffmetadata -i /tmp/ch.txt") && j.contains("-map_chapters 2"), "{j}");
        assert_eq!(c.side_files.len(), 1);

        let mut pr = base();
        pr.video = VideoAction::Drop;
        pr.audio = AudioAction::Drop;
        assert!(build(&pr).is_err());
        let mut pr = base();
        pr.output = "/o/x.m4a".into();
        assert!(build(&pr).is_err(), "video into m4a");
        pr.video = VideoAction::Drop;
        let j = build(&pr).unwrap().args.join(" ");
        assert!(!j.contains("0:V"));
        let mut pr = base();
        pr.trim_start = 60.0;
        pr.trim_end = 40.0;
        assert!(build(&pr).is_err());
        pr.duration_s = None;
        pr.trim_start = 1.0;
        pr.trim_end = 0.0;
        assert!(build(&pr).is_ok(), "trim start works without a duration");
        pr.trim_end = 1.0;
        assert!(build(&pr).is_err());
    }

    #[test]
    fn split() {
        assert_eq!(split_args(r#"-af "volume=2, loudnorm" -metadata 'title=My Game' C:\x\y"#).unwrap(), s(&["-af", "volume=2, loudnorm", "-metadata", "title=My Game", r"C:\x\y"]));
        assert_eq!(split_args("  ").unwrap(), Vec::<String>::new());
        assert_eq!(split_args(r#"-x """#).unwrap(), s(&["-x", ""]));
        assert!(split_args("\"open").is_err());
    }
}
