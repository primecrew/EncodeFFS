//! Mode 7: batch re-encode. Plans one `reencode::build` per input file with shared settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::common::*;
use super::reencode::{self, ReencodeParams, StreamRef};
use super::BuiltCommand;
use crate::util::{safe_name, stem};

/// Probe results for one batch input.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchInput {
    pub path: PathBuf,
    pub duration_s: f64,
    pub field_order: Option<String>,
    pub subtitle_streams: Vec<StreamRef>,
    pub has_attachments: bool,
}

impl BatchInput {
    pub fn interlaced(&self) -> bool {
        is_interlaced(self.field_order.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchSettings {
    /// "mp4" or "mkv"
    pub container: String,
    pub suffix: String,
    pub trim_start: f64,
    pub trim_end: f64,
    pub passthrough: Passthrough,
    /// Deinterlace files that ffprobe reports as interlaced (default yes, like the script).
    pub deinterlace: bool,
    pub range_convert: bool,
    pub video: VideoEncode,
    pub audio: AudioEncode,
    pub faststart: bool,
}
impl Default for BatchSettings {
    fn default() -> Self {
        BatchSettings {
            container: "mp4".into(),
            suffix: "_encoded".into(),
            trim_start: 0.0,
            trim_end: 0.0,
            passthrough: Passthrough::None,
            deinterlace: true,
            range_convert: false,
            video: VideoEncode::default(),
            audio: AudioEncode::default(),
            faststart: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchJob {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Per-job error log in the output folder (script: ffmpeg_error_<name>.log).
    pub log_file: PathBuf,
    pub cmd: BuiltCommand,
    pub out_duration: f64,
}

pub fn output_path(input: &Path, out_dir: &Path, suffix: &str, container: &str) -> PathBuf {
    out_dir.join(format!("{}{}.{}", stem(input), suffix, container))
}

pub fn plan(inputs: &[BatchInput], s: &BatchSettings, out_dir: &Path) -> Result<Vec<BatchJob>, String> {
    if inputs.is_empty() {
        return Err("No supported video files found.".into());
    }
    if s.container != "mp4" && s.container != "mkv" {
        return Err(format!("Invalid container choice: {}", s.container));
    }
    let mut jobs = Vec::new();
    for inp in inputs {
        let output = output_path(&inp.path, out_dir, &s.suffix, &s.container);
        if output == inp.path {
            return Err(format!("Output would overwrite the input: {}. Use a suffix or another output folder.", output.display()));
        }
        let pr = ReencodeParams {
            input: inp.path.clone(),
            output: output.clone(),
            duration_s: inp.duration_s,
            trim_start: s.trim_start,
            trim_end: s.trim_end,
            passthrough: s.passthrough,
            video: s.video.clone(),
            audio: s.audio.clone(),
            range_convert: s.range_convert,
            resize: None,
            deinterlace: s.deinterlace && inp.interlaced(),
            chapters: None,
            faststart: s.faststart,
            subtitle_streams: inp.subtitle_streams.clone(),
            has_attachments: inp.has_attachments,
        };
        let cmd = reencode::build(&pr).map_err(|e| format!("{}: {}", inp.path.display(), e))?;
        jobs.push(BatchJob {
            input: inp.path.clone(),
            log_file: out_dir.join(format!("ffmpeg_error_{}.log", safe_name(&stem(&inp.path)))),
            output,
            out_duration: pr.output_duration(),
            cmd,
        });
    }
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Vec<BatchInput> {
        vec![
            BatchInput { path: "/in/a.mkv".into(), duration_s: 60.0, field_order: Some("progressive".into()), ..Default::default() },
            BatchInput { path: "/in/b c.avi".into(), duration_s: 30.0, field_order: Some("tt".into()), ..Default::default() },
        ]
    }

    #[test]
    fn plans_each_file() {
        let jobs = plan(&inputs(), &BatchSettings::default(), Path::new("/out")).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].output, PathBuf::from("/out/a_encoded.mp4"));
        assert_eq!(jobs[1].output, PathBuf::from("/out/b c_encoded.mp4"));
        assert_eq!(jobs[1].log_file, PathBuf::from("/out/ffmpeg_error_b_c.log"));
        // deinterlace only on the interlaced file
        assert!(!jobs[0].cmd.args.iter().any(|a| a.contains("bwdif")));
        assert!(jobs[1].cmd.args.iter().any(|a| a.contains("bwdif")));
    }

    #[test]
    fn mkv_container_and_trim() {
        let s = BatchSettings { container: "mkv".into(), trim_start: 2.0, trim_end: 3.0, ..Default::default() };
        let jobs = plan(&inputs(), &s, Path::new("/out")).unwrap();
        assert_eq!(jobs[0].output, PathBuf::from("/out/a_encoded.mkv"));
        assert_eq!(jobs[1].out_duration, 25.0);
        assert!(jobs[1].cmd.args.windows(2).any(|w| w == ["-t", "25"]));
    }

    #[test]
    fn trim_exceeding_any_file_fails() {
        let s = BatchSettings { trim_start: 20.0, trim_end: 15.0, ..Default::default() };
        assert!(plan(&inputs(), &s, Path::new("/out")).is_err());
    }

    #[test]
    fn refuses_overwriting_input() {
        let s = BatchSettings { container: "mkv".into(), suffix: "".into(), ..Default::default() };
        assert!(plan(&inputs(), &s, Path::new("/in")).is_err());
    }

    #[test]
    fn no_deinterlace_when_disabled() {
        let s = BatchSettings { deinterlace: false, ..Default::default() };
        let jobs = plan(&inputs(), &s, Path::new("/out")).unwrap();
        assert!(!jobs[1].cmd.args.iter().any(|a| a.contains("bwdif")));
    }
}
