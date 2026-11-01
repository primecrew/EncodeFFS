//! Mode 6: mix all audio tracks into one AAC track in an MKV (video copied, chapters kept).

use std::path::PathBuf;

use super::{p, ArgVec, BuiltCommand};

#[derive(Debug, Clone, PartialEq)]
pub struct MixParams {
    pub input: PathBuf,
    /// Output path; forced to .mkv by `build`.
    pub output: PathBuf,
    pub audio_count: usize,
    pub bitrate_k: u32,
}

/// `[0:a:0][0:a:1]...amix=inputs=N:normalize=1[aout]`
pub fn mix_filter(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("[0:a:{i}]"));
    }
    s.push_str(&format!("amix=inputs={n}:normalize=1[aout]"));
    s
}

pub fn build(pr: &MixParams) -> Result<BuiltCommand, String> {
    if pr.audio_count < 2 {
        return Err(format!("Mixing requires at least two audio tracks. Found: {}", pr.audio_count));
    }
    let mut out = pr.output.clone();
    let mut warnings = vec![];
    if crate::util::ext_lower(&out) != "mkv" {
        out.set_extension("mkv");
        warnings.push(format!("Adjusted output file name to MKV: {}", out.display()));
    }
    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(&pr.input)];
    a.push_all(["-filter_complex".to_string(), mix_filter(pr.audio_count)]);
    a.push_all(["-map", "0:v", "-map", "[aout]", "-map_chapters", "0", "-c:v", "copy", "-c:a", "aac", "-b:a"]);
    a.push(format!("{}k", pr.bitrate_k));
    a.push(p(&out));
    Ok(BuiltCommand { args: a, warnings, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filter() {
        assert_eq!(mix_filter(3), "[0:a:0][0:a:1][0:a:2]amix=inputs=3:normalize=1[aout]");
    }
    #[test]
    fn args_and_validation() {
        let mut pr = MixParams { input: "/i.mkv".into(), output: "/o_mixed.mp4".into(), audio_count: 2, bitrate_k: 192 };
        let b = build(&pr).unwrap();
        assert_eq!(
            b.args.join(" "),
            "-y -i /i.mkv -filter_complex [0:a:0][0:a:1]amix=inputs=2:normalize=1[aout] -map 0:v -map [aout] -map_chapters 0 -c:v copy -c:a aac -b:a 192k /o_mixed.mkv"
        );
        assert_eq!(b.warnings.len(), 1);
        pr.audio_count = 1;
        assert!(build(&pr).is_err());
    }
}
