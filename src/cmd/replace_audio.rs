//! Mode 5: replace the primary audio with a WAV (copy video, AAC with gain/compressor).

use std::path::PathBuf;

use super::common::AudioEncode;
use super::{p, ArgVec, BuiltCommand};

#[derive(Debug, Clone, PartialEq)]
pub struct ReplaceAudioParams {
    pub video: PathBuf,
    pub wav: PathBuf,
    pub output: PathBuf,
    pub audio: AudioEncode,
}

pub fn build(pr: &ReplaceAudioParams) -> Result<BuiltCommand, String> {
    pr.audio.validate()?;
    let mut a: Vec<String> = vec!["-y".into(), "-i".into(), p(&pr.video), "-i".into(), p(&pr.wav)];
    a.push_all(["-map", "0:v", "-map", "1:a", "-c:v", "copy"]);
    a.extend(pr.audio.aac_args());
    if let Some(chain) = pr.audio.filter_chain() {
        a.push_all(["-filter:a".to_string(), chain]);
    }
    a.push(p(&pr.output));
    Ok(BuiltCommand { args: a, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::CompressorPreset;
    #[test]
    fn args() {
        let mut pr = ReplaceAudioParams {
            video: "/v.mp4".into(),
            wav: "/n.wav".into(),
            output: "/o.mp4".into(),
            audio: AudioEncode::default(),
        };
        assert_eq!(build(&pr).unwrap().args.join(" "), "-y -i /v.mp4 -i /n.wav -map 0:v -map 1:a -c:v copy -c:a aac -b:a 192k /o.mp4");
        pr.audio.gain_db = 6.0;
        pr.audio.compressor = CompressorPreset::Medium;
        let s = build(&pr).unwrap().args.join(" ");
        assert!(s.contains("-filter:a volume=6dB,acompressor=mode=downward"));
        pr.audio.gain_db = 31.0;
        assert!(build(&pr).is_err());
    }
}
