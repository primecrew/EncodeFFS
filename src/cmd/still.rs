//! Mode 2: MP4 from a still image + audio.

use std::path::PathBuf;

use super::common::*;
use super::{p, ArgVec, BuiltCommand};
use crate::util::fmt_num;

/// RGB/JPEG -> limited-range BT.709 YUV in a single scale step so the BT.709 tags are accurate.
pub const STILL_RANGE_FILTER: &str = "scale=out_range=tv:out_color_matrix=bt709,format=yuv420p";

#[derive(Debug, Clone, PartialEq)]
pub struct StillParams {
    pub image: PathBuf,
    pub audio: PathBuf,
    pub output: PathBuf,
    pub bitrate_k: u32,
    /// None = `-shortest` (use the audio length), Some(s) = `-t s`.
    pub duration_s: Option<f64>,
    pub faststart: bool,
    pub range_convert: bool,
}

pub fn build(pr: &StillParams) -> Result<BuiltCommand, String> {
    if let Some(d) = pr.duration_s {
        if d <= 0.0 {
            return Err(format!("Invalid duration: {d}"));
        }
    }
    let mut a: Vec<String> = Vec::new();
    a.push_all(["-y", "-loop", "1", "-i"]);
    a.push(p(&pr.image));
    a.push("-i".into());
    a.push(p(&pr.audio));
    a.push_all(["-c:v", "libx264", "-tune", "stillimage", "-pix_fmt", "yuv420p"]);
    if pr.range_convert {
        a.push_all(["-vf", STILL_RANGE_FILTER]);
        a.extend(color_tag_args());
    }
    a.push_all(["-c:a".to_string(), "aac".into(), "-b:a".into(), format!("{}k", pr.bitrate_k)]);
    match pr.duration_s {
        None => a.push("-shortest".into()),
        Some(d) => a.push_all(["-t".to_string(), fmt_num(d)]),
    }
    if pr.faststart {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> StillParams {
        StillParams {
            image: "/in/cover art.png".into(),
            audio: "/in/song.flac".into(),
            output: "/out/cover art_still.mp4".into(),
            bitrate_k: 192,
            duration_s: None,
            faststart: false,
            range_convert: false,
        }
    }
    #[test]
    fn shortest_default() {
        let b = build(&base()).unwrap();
        assert_eq!(
            b.args,
            vec!["-y", "-loop", "1", "-i", "/in/cover art.png", "-i", "/in/song.flac", "-c:v", "libx264", "-tune",
                 "stillimage", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest", "/out/cover art_still.mp4"]
        );
    }
    #[test]
    fn custom_duration_faststart_range() {
        let mut pr = base();
        pr.duration_s = Some(300.5);
        pr.faststart = true;
        pr.range_convert = true;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-t 300.5 -movflags +faststart /out/cover art_still.mp4"));
        assert!(s.contains(STILL_RANGE_FILTER));
        assert!(s.contains("-color_range tv -colorspace bt709"));
        pr.duration_s = Some(0.0);
        assert!(build(&pr).is_err());
    }
}
