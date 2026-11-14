//! Mode 3: add a commentary track (copy video + original audio, add AAC commentary).

use std::path::PathBuf;

use super::common::faststart_args;
use super::reencode::is_mp4_like;
use super::{p, ArgVec, BuiltCommand};

#[derive(Debug, Clone, PartialEq)]
pub struct CommentaryParams {
    pub video: PathBuf,
    pub commentary: PathBuf,
    pub output: PathBuf,
    pub title: String,
    pub bitrate_k: u32,
    pub faststart: bool,
    /// Number of audio streams already in the video (from ffprobe).
    pub original_audio_count: usize,
}

pub fn build(pr: &CommentaryParams) -> Result<BuiltCommand, String> {
    // The commentary becomes the audio stream right after the originals.
    // (The script hard-coded a:1, which is wrong when the source has 0 or 2+ tracks.)
    let n = pr.original_audio_count;
    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(&pr.video), "-i".into(), p(&pr.commentary)];
    a.push_all(["-map", "0:v"]);
    if n > 0 {
        a.push_all(["-map", "0:a"]);
    }
    a.push_all(["-map", "1:a", "-c:v", "copy", "-c:a", "copy"]);
    a.push_all([format!("-c:a:{n}"), "aac".into(), format!("-b:a:{n}"), format!("{}k", pr.bitrate_k)]);
    let title = if pr.title.trim().is_empty() { "Additional audio" } else { pr.title.trim() };
    a.push_all([format!("-metadata:s:a:{n}"), format!("title={title}")]);
    if is_mp4_like(&pr.output) {
        // MP4 has no per-track "title"; players show the handler name instead.
        a.push_all([format!("-metadata:s:a:{n}"), format!("handler_name={title}")]);
    }
    if pr.faststart && is_mp4_like(&pr.output) {
        a.extend(faststart_args());
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> CommentaryParams {
        CommentaryParams {
            video: "/in/v.mp4".into(),
            commentary: "/in/c.wav".into(),
            output: "/out/v_with_commentary.mp4".into(),
            title: "Commentary Track".into(),
            bitrate_k: 160,
            faststart: true,
            original_audio_count: 1,
        }
    }
    #[test]
    fn one_original_track() {
        let b = build(&base()).unwrap();
        assert_eq!(
            b.args,
            vec!["-y", "-i", "/in/v.mp4", "-i", "/in/c.wav", "-map", "0:v", "-map", "0:a", "-map", "1:a", "-c:v", "copy",
                 "-c:a", "copy", "-c:a:1", "aac", "-b:a:1", "160k", "-metadata:s:a:1", "title=Commentary Track",
                 "-metadata:s:a:1", "handler_name=Commentary Track", "-movflags", "+faststart", "/out/v_with_commentary.mp4"]
        );
    }
    #[test]
    fn index_follows_original_count() {
        let mut pr = base();
        pr.original_audio_count = 2;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-c:a:2 aac -b:a:2 160k -metadata:s:a:2 title=Commentary Track"));
        pr.original_audio_count = 0;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(!s.contains("0:a"));
        assert!(s.contains("-c:a:0 aac"));
    }
}
