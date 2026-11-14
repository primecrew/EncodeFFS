//! Mode 4: extract audio to PCM WAV.

use std::path::PathBuf;

use super::{p, BuiltCommand};

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractParams {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Detected sample rate, or the fallback from settings.
    pub sample_rate: u32,
    /// When set, map this 0-based audio stream (`-map 0:a:N`).
    /// `None` leaves stream selection to FFmpeg (today's command, no `-map`).
    pub map_audio: Option<usize>,
}

/// Strip characters that `check_output` rejects, plus other path-unsafe bits.
pub fn sanitize_title(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| c == '.' || c == '_' || c.is_whitespace()).to_string()
}

/// `<name> - Track 2 - Mic.wav` (title omitted when missing or empty after sanitizing).
pub fn wav_name_for_track(base: &str, track_1based: usize, title: Option<&str>) -> String {
    let mut name = format!("{base} - Track {track_1based}");
    if let Some(t) = title.map(sanitize_title).filter(|s| !s.is_empty()) {
        name.push_str(" - ");
        name.push_str(&t);
    }
    name.push_str(".wav");
    name
}

pub fn build(pr: &ExtractParams) -> Result<BuiltCommand, String> {
    if pr.sample_rate == 0 {
        return Err("Invalid sample rate".into());
    }
    let mut args = vec!["-y".into(), "-i".into(), p(&pr.input)];
    if let Some(n) = pr.map_audio {
        args.push("-map".into());
        args.push(format!("0:a:{n}"));
    }
    args.extend([
        "-vn".into(),
        "-acodec".into(),
        "pcm_s16le".into(),
        "-ar".into(),
        pr.sample_rate.to_string(),
        "-ac".into(),
        "2".into(),
        p(&pr.output),
    ]);
    Ok(BuiltCommand { args, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(map_audio: Option<usize>) -> ExtractParams {
        ExtractParams { input: "/a b/v.mkv".into(), output: "/o/v_audio.wav".into(), sample_rate: 44100, map_audio }
    }

    #[test]
    fn wav_args_default_is_byte_identical() {
        let b = build(&params(None)).unwrap();
        assert_eq!(b.args.join(" "), "-y -i /a b/v.mkv -vn -acodec pcm_s16le -ar 44100 -ac 2 /o/v_audio.wav");
        assert!(!b.args.iter().any(|a| a == "-map" || a.starts_with("0:a:")));
    }

    #[test]
    fn wav_args_specific_track() {
        let b = build(&ExtractParams { output: "/o/v - Track 3 - Music.wav".into(), map_audio: Some(2), sample_rate: 48000, ..params(None) }).unwrap();
        assert_eq!(b.args.join(" "), "-y -i /a b/v.mkv -map 0:a:2 -vn -acodec pcm_s16le -ar 48000 -ac 2 /o/v - Track 3 - Music.wav");
    }

    #[test]
    fn track_wav_names() {
        assert_eq!(wav_name_for_track("clip_audio", 2, Some("Mic")), "clip_audio - Track 2 - Mic.wav");
        assert_eq!(wav_name_for_track("clip", 1, None), "clip - Track 1.wav");
        assert_eq!(wav_name_for_track("clip", 3, Some("  Music/Lead?  ")), "clip - Track 3 - Music_Lead.wav");
        assert_eq!(sanitize_title("  ..a:b*c  "), "a_b_c");
    }
}
