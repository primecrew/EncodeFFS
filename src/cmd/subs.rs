//! Mode 8: copy selected subtitle tracks from a source video into a destination video.
//! Input 0 = subtitle source, input 1 = destination (video/audio/metadata/chapters come from it).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::common::*;
use super::reencode::is_mp4_like;
use super::{p, ArgVec, BuiltCommand};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SubStreamInfo {
    pub index: u32,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
}

impl SubStreamInfo {
    pub fn describe(&self) -> String {
        let mut s = format!(
            "Stream {}: codec={}, language={}",
            self.index,
            self.codec,
            self.language.as_deref().unwrap_or("und")
        );
        if let Some(t) = self.title.as_deref().filter(|t| !t.is_empty()) {
            s.push_str(&format!(", title={t}"));
        }
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mp4SubHandling {
    /// Convert subtitle tracks to mov_text (script option 1, default).
    #[default]
    ConvertMovText,
    /// Try copying the codec as-is (script option 2).
    TryCopy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubsParams {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub output: PathBuf,
    pub source_streams: Vec<SubStreamInfo>,
    /// Absolute stream indices (in the source) chosen by the user.
    pub selected: Vec<u32>,
    pub dest_streams: Vec<SubStreamInfo>,
    pub keep_existing: bool,
    pub copy_only: bool,
    pub video: VideoEncode,
    pub audio_bitrate_k: u32,
    pub range_convert: bool,
    pub mp4_handling: Mp4SubHandling,
    pub faststart: bool,
}

/// Which subtitle codec option to use and a warning if MP4 may not take the tracks.
pub fn subtitle_codec(mp4: bool, streams: &[&SubStreamInfo], handling: Mp4SubHandling) -> (&'static str, Option<String>) {
    if !mp4 {
        return ("copy", None);
    }
    let mut bad: Vec<&str> = streams.iter().filter(|s| !mp4_sub_compatible(&s.codec)).map(|s| s.codec.as_str()).collect();
    bad.sort();
    bad.dedup();
    let warning = (!bad.is_empty()).then(|| {
        format!(
            "MP4 subtitle transfer may fail or lose subtitle behavior for these codec(s): {}. Image-based subtitles cannot be converted to mov_text; use MKV output to keep them.",
            bad.join(", ")
        )
    });
    let codec = match handling {
        Mp4SubHandling::ConvertMovText => "mov_text",
        Mp4SubHandling::TryCopy => "copy",
    };
    (codec, warning)
}

pub fn build(pr: &SubsParams) -> Result<BuiltCommand, String> {
    if pr.source_streams.is_empty() {
        return Err(format!("No subtitle streams were found in: {}", pr.source.display()));
    }
    if pr.selected.is_empty() {
        return Err("Select at least one subtitle track to copy.".into());
    }
    let selected: Vec<&SubStreamInfo> = pr
        .selected
        .iter()
        .map(|i| {
            pr.source_streams
                .iter()
                .find(|s| s.index == *i)
                .ok_or_else(|| format!("Stream {i} is not a subtitle stream in the source"))
        })
        .collect::<Result<_, _>>()?;
    let mp4 = is_mp4_like(&pr.output);
    let mut involved = selected.clone();
    if pr.keep_existing {
        involved.extend(pr.dest_streams.iter());
    }
    let (codec, warning) = subtitle_codec(mp4, &involved, pr.mp4_handling);

    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(&pr.source), "-i".into(), p(&pr.destination)];
    a.push_all(["-map", "1:v?", "-map", "1:a?"]);
    if pr.keep_existing {
        a.push_all(["-map", "1:s?"]);
    }
    for s in &selected {
        a.push_all(["-map".to_string(), format!("0:{}", s.index)]);
    }
    a.push_all(["-map_metadata", "1", "-map_chapters", "1"]);
    if pr.copy_only {
        a.push_all(["-c:v", "copy", "-c:a", "copy"]);
    } else {
        let filter = pr.range_convert.then_some(RANGE_FILTER);
        a.extend(x264_args(&pr.video, filter, pr.range_convert));
        a.push_all(["-c:a".to_string(), "aac".into(), "-b:a".into(), format!("{}k", pr.audio_bitrate_k)]);
    }
    a.push_all(["-c:s", codec]);
    if mp4 && pr.faststart {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, warnings: warning.into_iter().collect(), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SubsParams {
        SubsParams {
            source: "/s/src.mkv".into(),
            destination: "/d/dst.mp4".into(),
            output: "/o/dst_with_subs.mkv".into(),
            source_streams: vec![
                SubStreamInfo { index: 2, codec: "subrip".into(), language: Some("eng".into()), title: None },
                SubStreamInfo { index: 3, codec: "hdmv_pgs_subtitle".into(), language: Some("ger".into()), title: Some("Forced".into()) },
            ],
            selected: vec![2, 3],
            dest_streams: vec![],
            keep_existing: true,
            copy_only: true,
            video: VideoEncode::default(),
            audio_bitrate_k: 192,
            range_convert: false,
            mp4_handling: Mp4SubHandling::ConvertMovText,
            faststart: true,
        }
    }

    #[test]
    fn mkv_copy() {
        let b = build(&base()).unwrap();
        assert_eq!(
            b.args.join(" "),
            "-y -i /s/src.mkv -i /d/dst.mp4 -map 1:v? -map 1:a? -map 1:s? -map 0:2 -map 0:3 -map_metadata 1 -map_chapters 1 -c:v copy -c:a copy -c:s copy /o/dst_with_subs.mkv"
        );
        assert!(b.warnings.is_empty());
    }

    #[test]
    fn mp4_movtext_with_warning() {
        let mut pr = base();
        pr.output = "/o/x.mp4".into();
        pr.keep_existing = false;
        let b = build(&pr).unwrap();
        let s = b.args.join(" ");
        assert!(s.contains("-c:s mov_text -movflags +faststart /o/x.mp4"), "{s}");
        assert!(!s.contains("1:s?"));
        assert_eq!(b.warnings.len(), 1);
        assert!(b.warnings[0].contains("hdmv_pgs_subtitle"));
        // only text subs selected: no warning
        pr.selected = vec![2];
        assert!(build(&pr).unwrap().warnings.is_empty());
        pr.mp4_handling = Mp4SubHandling::TryCopy;
        assert!(build(&pr).unwrap().args.join(" ").contains("-c:s copy"));
    }

    #[test]
    fn reencode_path() {
        let mut pr = base();
        pr.copy_only = false;
        pr.range_convert = true;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-c:v libx264 -crf 22 -preset slow -pix_fmt yuv420p -vf scale=in_range=full:out_range=tv -color_range tv"));
        assert!(s.contains("-c:a aac -b:a 192k -c:s copy"));
    }

    #[test]
    fn selection_validation() {
        let mut pr = base();
        pr.selected = vec![];
        assert!(build(&pr).is_err());
        pr.selected = vec![9];
        assert!(build(&pr).is_err());
        pr.source_streams.clear();
        assert!(build(&pr).is_err());
    }
}
