//! ffprobe JSON parsing (pure) and invocation.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::cmd::common::is_interlaced;
use crate::cmd::reencode::StreamRef;
use crate::cmd::subs::SubStreamInfo;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamInfo {
    pub index: u32,
    pub codec_type: String,
    pub codec_name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub field_order: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaInfo {
    pub streams: Vec<StreamInfo>,
    pub format_duration: Option<f64>,
}

/// The facts the re-encode modes need (script: Get-VideoInfo).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoSummary {
    pub duration_s: f64,
    pub width: u32,
    pub height: u32,
    pub audio_count: usize,
    pub field_order: Option<String>,
    pub interlaced: bool,
}

#[derive(Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}
#[derive(Deserialize)]
struct RawFormat {
    duration: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct RawStream {
    index: Option<u32>,
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<serde_json::Value>,
    height: Option<serde_json::Value>,
    duration: Option<serde_json::Value>,
    field_order: Option<String>,
    sample_rate: Option<serde_json::Value>,
    channels: Option<serde_json::Value>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

/// ffprobe emits numbers sometimes as JSON numbers, sometimes as strings ("10.000000", "N/A").
fn num(v: &Option<serde_json::Value>) -> Option<f64> {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

fn tag(tags: &HashMap<String, String>, key: &str) -> Option<String> {
    tags.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()).filter(|v| !v.is_empty())
}

pub fn parse_probe_json(json: &str) -> Result<MediaInfo, String> {
    let raw: RawProbe = serde_json::from_str(json).map_err(|e| format!("Could not parse ffprobe output: {e}"))?;
    let streams = raw
        .streams
        .into_iter()
        .enumerate()
        .map(|(i, s)| StreamInfo {
            index: s.index.unwrap_or(i as u32),
            codec_type: s.codec_type.unwrap_or_default(),
            codec_name: s.codec_name.unwrap_or_default(),
            width: num(&s.width).map(|v| v as u32),
            height: num(&s.height).map(|v| v as u32),
            duration: num(&s.duration),
            field_order: s.field_order,
            sample_rate: num(&s.sample_rate).map(|v| v as u32),
            channels: num(&s.channels).map(|v| v as u32),
            language: tag(&s.tags, "language"),
            title: tag(&s.tags, "title"),
        })
        .collect();
    Ok(MediaInfo { streams, format_duration: raw.format.and_then(|f| num(&f.duration)) })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

impl MediaInfo {
    pub fn of_type<'a>(&'a self, t: &'a str) -> impl Iterator<Item = &'a StreamInfo> + 'a {
        self.streams.iter().filter(move |s| s.codec_type == t)
    }
    pub fn video(&self) -> Option<&StreamInfo> {
        self.of_type("video").next()
    }
    pub fn audio_count(&self) -> usize {
        self.of_type("audio").count()
    }
    pub fn has_attachments(&self) -> bool {
        self.of_type("attachment").next().is_some()
    }
    /// Duration: video stream duration, else container duration (rounded to 0.01 s like the script).
    pub fn duration(&self) -> Option<f64> {
        self.video().and_then(|v| v.duration).or(self.format_duration).map(round2).filter(|d| *d > 0.0)
    }
    /// Container duration, falling back to any stream (used for audio-only files).
    pub fn any_duration(&self) -> Option<f64> {
        self.format_duration.or_else(|| self.streams.iter().filter_map(|s| s.duration).fold(None, |a, b| Some(a.map_or(b, |a: f64| a.max(b))))).map(round2).filter(|d| *d > 0.0)
    }
    pub fn subtitle_refs(&self) -> Vec<StreamRef> {
        self.of_type("subtitle").map(|s| StreamRef { index: s.index, codec: s.codec_name.clone() }).collect()
    }
    pub fn subtitle_infos(&self) -> Vec<SubStreamInfo> {
        self.of_type("subtitle")
            .map(|s| SubStreamInfo { index: s.index, codec: s.codec_name.clone(), language: s.language.clone(), title: s.title.clone() })
            .collect()
    }
    pub fn first_audio_sample_rate(&self) -> Option<u32> {
        self.of_type("audio").next().and_then(|a| a.sample_rate)
    }

    /// Script's Get-VideoInfo validation.
    pub fn video_summary(&self) -> Result<VideoSummary, String> {
        let v = self.video().ok_or("No video stream found")?;
        let duration_s = self.duration().ok_or("Invalid duration: could not determine a positive duration")?;
        let (w, h) = (v.width.unwrap_or(0), v.height.unwrap_or(0));
        if w == 0 || h == 0 {
            return Err(format!("Invalid resolution: width={w}, height={h}"));
        }
        Ok(VideoSummary {
            duration_s,
            width: w,
            height: h,
            audio_count: self.audio_count(),
            field_order: v.field_order.clone(),
            interlaced: is_interlaced(v.field_order.as_deref()),
        })
    }
}

/// Arguments for one ffprobe call that returns everything the modes need.
pub fn probe_args(path: &Path) -> Vec<String> {
    vec![
        "-v".into(),
        "error".into(),
        "-show_entries".into(),
        "stream=index,codec_type,codec_name,width,height,duration,field_order,sample_rate,channels:stream_tags=language,title:format=duration".into(),
        "-of".into(),
        "json".into(),
        path.to_string_lossy().into_owned(),
    ]
}

/// Run ffprobe (no console window) and parse the result.
pub fn probe(ffprobe: &Path, file: &Path) -> Result<MediaInfo, String> {
    let out = super::hidden_command(ffprobe)
        .args(probe_args(file))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not run ffprobe ({}): {e}", ffprobe.display()))?;
    if !out.status.success() {
        return Err(format!(
            "ffprobe failed for {}: {}",
            file.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse_probe_json(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MKV: &str = r#"{
      "programs": [],
      "streams": [
        {"index":0,"codec_name":"h264","codec_type":"video","width":1920,"height":1080,"field_order":"tt","tags":{"DURATION":"00:00:10.000"}},
        {"index":1,"codec_name":"aac","codec_type":"audio","sample_rate":"48000","channels":2,"tags":{"language":"eng"}},
        {"index":2,"codec_name":"opus","codec_type":"audio","sample_rate":"48000","channels":2},
        {"index":3,"codec_name":"subrip","codec_type":"subtitle","tags":{"language":"ger","title":"Forced"}},
        {"index":4,"codec_name":"ttf","codec_type":"attachment"}
      ],
      "format": {"duration":"10.033000"}
    }"#;

    #[test]
    fn parse_mkv() {
        let m = parse_probe_json(MKV).unwrap();
        let s = m.video_summary().unwrap();
        assert_eq!(s.duration_s, 10.03);
        assert_eq!((s.width, s.height, s.audio_count), (1920, 1080, 2));
        assert!(s.interlaced);
        assert!(m.has_attachments());
        assert_eq!(m.first_audio_sample_rate(), Some(48000));
        let subs = m.subtitle_infos();
        assert_eq!(subs[0].index, 3);
        assert_eq!(subs[0].describe(), "Stream 3: codec=subrip, language=ger, title=Forced");
    }

    #[test]
    fn stream_duration_preferred() {
        let j = r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":480,"duration":"5.004","field_order":"progressive"}],"format":{"duration":"6"}}"#;
        let s = parse_probe_json(j).unwrap().video_summary().unwrap();
        assert_eq!(s.duration_s, 5.0);
        assert!(!s.interlaced);
    }

    #[test]
    fn errors() {
        assert!(parse_probe_json("{}").unwrap().video_summary().is_err());
        let j = r#"{"streams":[{"index":0,"codec_type":"video","width":0,"height":0,"duration":"5"}]}"#;
        assert!(parse_probe_json(j).unwrap().video_summary().is_err());
        assert!(parse_probe_json("not json").is_err());
    }

    #[test]
    fn audio_only_duration() {
        let j = r#"{"streams":[{"index":0,"codec_type":"audio","sample_rate":"44100"}],"format":{"duration":"12.345"}}"#;
        assert_eq!(parse_probe_json(j).unwrap().any_duration(), Some(12.35));
    }
}
