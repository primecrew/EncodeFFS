//! ffprobe JSON parsing (pure) and invocation.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::cmd::common::{is_interlaced, ColorRange};
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
    pub profile: Option<String>,
    pub pix_fmt: Option<String>,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    /// Average frame rate (frames/s) for video streams.
    pub fps: Option<f64>,
    pub bit_rate: Option<u64>,
    pub channel_layout: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// Cover art stored as a video stream.
    pub attached_pic: bool,
}

/// A chapter already present in the file (seconds).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProbeChapter {
    pub start: f64,
    pub end: f64,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaInfo {
    pub streams: Vec<StreamInfo>,
    pub format_duration: Option<f64>,
    pub chapters: Vec<ProbeChapter>,
    pub format_name: Option<String>,
    pub format_long_name: Option<String>,
    pub size: Option<u64>,
    pub bit_rate: Option<u64>,
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
    #[serde(default)]
    chapters: Vec<RawChapter>,
}
#[derive(Deserialize)]
struct RawFormat {
    duration: Option<serde_json::Value>,
    format_name: Option<String>,
    format_long_name: Option<String>,
    size: Option<serde_json::Value>,
    bit_rate: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct RawChapter {
    start_time: Option<serde_json::Value>,
    end_time: Option<serde_json::Value>,
    #[serde(default)]
    tags: HashMap<String, String>,
}
#[derive(Deserialize, Default)]
struct RawDisposition {
    #[serde(default)]
    default: i64,
    #[serde(default)]
    forced: i64,
    #[serde(default)]
    attached_pic: i64,
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
    profile: Option<String>,
    pix_fmt: Option<String>,
    color_range: Option<String>,
    color_space: Option<String>,
    color_primaries: Option<String>,
    color_transfer: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    bit_rate: Option<serde_json::Value>,
    channel_layout: Option<String>,
    #[serde(default)]
    disposition: RawDisposition,
}

/// "30000/1001" -> 29.97; "0/0" -> None.
pub fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    (n > 0.0 && d > 0.0).then(|| n / d)
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
            profile: s.profile,
            pix_fmt: s.pix_fmt,
            color_range: s.color_range.filter(|v| !v.trim().is_empty()),
            color_space: s.color_space.filter(|v| !v.trim().is_empty()),
            color_primaries: s.color_primaries.filter(|v| !v.trim().is_empty()),
            color_transfer: s.color_transfer.filter(|v| !v.trim().is_empty()),
            fps: s.avg_frame_rate.as_deref().and_then(parse_rate).or_else(|| s.r_frame_rate.as_deref().and_then(parse_rate)),
            bit_rate: num(&s.bit_rate).map(|v| v as u64),
            channel_layout: s.channel_layout,
            default: s.disposition.default != 0,
            forced: s.disposition.forced != 0,
            attached_pic: s.disposition.attached_pic != 0,
        })
        .collect();
    let chapters = raw
        .chapters
        .into_iter()
        .map(|c| ProbeChapter { start: num(&c.start_time).unwrap_or(0.0), end: num(&c.end_time).unwrap_or(0.0), title: tag(&c.tags, "title").unwrap_or_default() })
        .collect();
    let f = raw.format;
    Ok(MediaInfo {
        streams,
        format_duration: f.as_ref().and_then(|f| num(&f.duration)),
        chapters,
        format_name: f.as_ref().and_then(|f| f.format_name.clone()),
        format_long_name: f.as_ref().and_then(|f| f.format_long_name.clone()),
        size: f.as_ref().and_then(|f| num(&f.size)).map(|v| v as u64),
        bit_rate: f.as_ref().and_then(|f| num(&f.bit_rate)).map(|v| v as u64),
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

impl MediaInfo {
    pub fn of_type<'a>(&'a self, t: &'a str) -> impl Iterator<Item = &'a StreamInfo> + 'a {
        self.streams.iter().filter(move |s| s.codec_type == t)
    }
    /// First real video stream (cover art is skipped).
    pub fn video(&self) -> Option<&StreamInfo> {
        self.of_type("video").find(|s| !s.attached_pic)
    }
    pub fn has_chapters(&self) -> bool {
        !self.chapters.is_empty()
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

    /// Tagged colour range of the first real video stream, when ffprobe reported a known value.
    pub fn video_color_range(&self) -> Option<ColorRange> {
        self.video().and_then(|v| v.color_range.as_deref().and_then(ColorRange::parse))
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
        "stream=index,codec_type,codec_name,profile,width,height,duration,field_order,sample_rate,channels,channel_layout,avg_frame_rate,r_frame_rate,pix_fmt,bit_rate,color_range,color_space,color_primaries,color_transfer:stream_tags=language,title:stream_disposition=default,forced,attached_pic:format=duration,format_name,format_long_name,size,bit_rate:chapter=start_time,end_time:chapter_tags=title".into(),
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

    #[test]
    fn chapters_fps_format() {
        let j = r#"{"streams":[
            {"index":0,"codec_type":"video","codec_name":"h264","profile":"High","width":1280,"height":720,"pix_fmt":"yuv420p","avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001","disposition":{"default":1,"forced":0,"attached_pic":0}},
            {"index":1,"codec_type":"video","codec_name":"mjpeg","width":300,"height":300,"avg_frame_rate":"0/0","disposition":{"attached_pic":1}},
            {"index":2,"codec_type":"audio","codec_name":"aac","channel_layout":"stereo","bit_rate":"192000"}],
          "chapters":[{"start_time":"0.000000","end_time":"2.500000","tags":{"title":"x=y;z#w\\v ü"}},{"start_time":"2.5","end_time":"4"}],
          "format":{"duration":"4.000","format_name":"mov,mp4,m4a,3gp,3g2,mj2","format_long_name":"QuickTime / MOV","size":"12345","bit_rate":"999"}}"#;
        let m = parse_probe_json(j).unwrap();
        assert_eq!(m.chapters.len(), 2);
        assert_eq!(m.chapters[0], ProbeChapter { start: 0.0, end: 2.5, title: "x=y;z#w\\v ü".into() });
        assert_eq!(m.chapters[1].title, "");
        assert!(m.has_chapters());
        let v = m.video().unwrap();
        assert_eq!(v.index, 0, "cover art skipped");
        assert!((v.fps.unwrap() - 29.97).abs() < 0.01);
        assert!(v.default);
        assert_eq!(m.streams[2].bit_rate, Some(192000));
        assert_eq!((m.size, m.bit_rate, m.format_long_name.as_deref()), (Some(12345), Some(999), Some("QuickTime / MOV")));
        assert_eq!(parse_rate("25"), Some(25.0));
        assert_eq!(parse_rate("0/0"), None);
    }

    #[test]
    fn color_tags_are_parsed_and_unknown_is_untagged() {
        let tv = r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":360,"duration":"1","color_range":"tv","color_space":"bt709","color_primaries":"bt709","color_transfer":"bt709"}]}"#;
        let m = parse_probe_json(tv).unwrap();
        let v = m.video().unwrap();
        assert_eq!(v.color_range.as_deref(), Some("tv"));
        assert_eq!(v.color_space.as_deref(), Some("bt709"));
        assert_eq!(v.color_primaries.as_deref(), Some("bt709"));
        assert_eq!(v.color_transfer.as_deref(), Some("bt709"));
        assert_eq!(m.video_color_range(), Some(ColorRange::Limited));

        let pc = r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":360,"duration":"1","color_range":"pc"}]}"#;
        assert_eq!(parse_probe_json(pc).unwrap().video_color_range(), Some(ColorRange::Full));

        let unknown = r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":360,"duration":"1","color_range":"unknown"}]}"#;
        assert_eq!(parse_probe_json(unknown).unwrap().video_color_range(), None);

        let missing = r#"{"streams":[{"index":0,"codec_type":"video","width":640,"height":360,"duration":"1"}]}"#;
        assert_eq!(parse_probe_json(missing).unwrap().video_color_range(), None);

        let args = probe_args(Path::new("clip.mp4")).join(" ");
        assert!(args.contains("color_range"), "{args}");
        assert!(args.contains("color_space"), "{args}");
        assert!(args.contains("color_primaries"), "{args}");
        assert!(args.contains("color_transfer"), "{args}");
    }
}
