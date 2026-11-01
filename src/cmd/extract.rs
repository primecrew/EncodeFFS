//! Mode 4: extract audio to PCM WAV.

use std::path::PathBuf;

use super::{p, BuiltCommand};

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractParams {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Detected sample rate, or the fallback from settings.
    pub sample_rate: u32,
}

pub fn build(pr: &ExtractParams) -> Result<BuiltCommand, String> {
    if pr.sample_rate == 0 {
        return Err("Invalid sample rate".into());
    }
    Ok(BuiltCommand {
        args: vec![
            "-y".into(), "-i".into(), p(&pr.input), "-vn".into(), "-acodec".into(), "pcm_s16le".into(),
            "-ar".into(), pr.sample_rate.to_string(), "-ac".into(), "2".into(), p(&pr.output),
        ],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_args() {
        let b = build(&ExtractParams { input: "/a b/v.mkv".into(), output: "/o/v_audio.wav".into(), sample_rate: 44100 }).unwrap();
        assert_eq!(b.args.join(" "), "-y -i /a b/v.mkv -vn -acodec pcm_s16le -ar 44100 -ac 2 /o/v_audio.wav");
    }
}
