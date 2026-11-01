//! HandBrake-style "Source" panel: an ffprobe summary of the selected preset's input.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, RichText, Ui};
use ffenc::cmd::chapters::fmt_ms;
use ffenc::ffmpeg::probe::{MediaInfo, StreamInfo};

use super::common::{hint, ProbeSlot};

#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub general: Vec<(String, String)>,
    pub video: Vec<(String, String)>,
    pub audio: Vec<String>,
    pub subtitles: Vec<String>,
    pub chapters: Vec<String>,
    pub attachments: usize,
}

pub fn fmt_size(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GiB", b / 1024.0 / 1024.0 / 1024.0)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.0} KiB", b / 1024.0)
    } else {
        format!("{b} B")
    }
}

pub fn fmt_rate(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.1} Mb/s", bps as f64 / 1e6)
    } else {
        format!("{} kb/s", (bps as f64 / 1e3).round())
    }
}

fn fps_text(f: f64) -> String {
    let r = (f * 1000.0).round() / 1000.0;
    if (r - r.round()).abs() < 1e-9 {
        format!("{}", r.round() as i64)
    } else {
        format!("{r}")
    }
}

fn tags(s: &StreamInfo) -> String {
    let mut t = String::new();
    if let Some(l) = s.language.as_deref().filter(|l| !l.is_empty() && *l != "und") {
        t.push_str(&format!(" [{l}]"));
    }
    if let Some(n) = s.title.as_deref().filter(|n| !n.is_empty()) {
        t.push_str(&format!(" \"{n}\""));
    }
    if s.default {
        t.push_str(" (default)");
    }
    if s.forced {
        t.push_str(" (forced)");
    }
    t
}

pub fn interlace_text(field_order: Option<&str>) -> String {
    match field_order {
        None | Some("") | Some("unknown") => "unknown (not reported)".into(),
        Some("progressive") => "progressive".into(),
        Some("tt") | Some("tb") => format!("interlaced, top field first ({})", field_order.unwrap()),
        Some("bb") | Some("bt") => format!("interlaced, bottom field first ({})", field_order.unwrap()),
        Some(o) => format!("interlaced ({o})"),
    }
}

pub fn summarize(path: &Path, i: &MediaInfo) -> Summary {
    let mut s = Summary::default();
    s.general.push(("File".into(), path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    if let Some(f) = i.format_long_name.as_ref().or(i.format_name.as_ref()) {
        s.general.push(("Container".into(), f.clone()));
    }
    if let Some(d) = i.any_duration() {
        s.general.push(("Duration".into(), fmt_ms((d * 1000.0).round() as i64)));
    }
    if let Some(b) = i.size {
        s.general.push(("Size".into(), fmt_size(b)));
    }
    if let Some(b) = i.bit_rate {
        s.general.push(("Bitrate".into(), fmt_rate(b)));
    }
    if let Some(v) = i.video() {
        let mut codec = v.codec_name.clone();
        if let Some(p) = v.profile.as_deref().filter(|p| !p.is_empty()) {
            codec.push_str(&format!(" ({p})"));
        }
        s.video.push(("Codec".into(), codec));
        if let (Some(w), Some(h)) = (v.width, v.height) {
            s.video.push(("Resolution".into(), format!("{w} x {h}")));
        }
        if let Some(f) = v.fps {
            s.video.push(("Frame rate".into(), format!("{} fps", fps_text(f))));
        }
        if let Some(p) = &v.pix_fmt {
            s.video.push(("Pixel format".into(), p.clone()));
        }
        s.video.push(("Scan".into(), interlace_text(v.field_order.as_deref())));
        if let Some(b) = v.bit_rate {
            s.video.push(("Bitrate".into(), fmt_rate(b)));
        }
    }
    for (n, a) in i.of_type("audio").enumerate() {
        let mut l = format!("{}. {}", n + 1, a.codec_name);
        if let Some(c) = a.channel_layout.as_deref().filter(|c| !c.is_empty()) {
            l.push_str(&format!(", {c}"));
        } else if let Some(c) = a.channels {
            l.push_str(&format!(", {c} ch"));
        }
        if let Some(r) = a.sample_rate {
            l.push_str(&format!(", {} kHz", r as f64 / 1000.0));
        }
        if let Some(b) = a.bit_rate {
            l.push_str(&format!(", {}", fmt_rate(b)));
        }
        l.push_str(&tags(a));
        s.audio.push(l);
    }
    for (n, t) in i.of_type("subtitle").enumerate() {
        s.subtitles.push(format!("{}. {}{}", n + 1, t.codec_name, tags(t)));
    }
    for (n, c) in i.chapters.iter().enumerate() {
        let title = if c.title.is_empty() { format!("Chapter {}", n + 1) } else { c.title.replace('\n', " ") };
        s.chapters.push(format!("{}  {}", fmt_ms((c.start * 1000.0).round() as i64), title));
    }
    s.attachments = i.of_type("attachment").count();
    s
}

/// Probes the current preset's input and draws the summary.
#[derive(Default)]
pub struct SourcePanel {
    slot: ProbeSlot,
    path: Option<PathBuf>,
}

impl SourcePanel {
    pub fn ui(&mut self, ui: &mut Ui, input: Option<PathBuf>, ffprobe: Option<&Path>) {
        let p = input.clone().unwrap_or_default();
        self.slot.update(ui.ctx(), ffprobe, &p.to_string_lossy());
        self.path = input;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Source").strong());
        });
        match ffprobe {
            Some(fp) => {
                hint(ui, format!("Read with ffprobe (same folder as ffmpeg):\n{}", fp.display()));
            }
            None => {
                ui.colored_label(Color32::LIGHT_RED, "ffprobe not found – it must be in the same folder as ffmpeg (Help › Locate FFmpeg folder).");
            }
        }
        ui.separator();
        let Some(path) = self.path.clone() else {
            hint(ui, "Choose or drop an input file to see its details.");
            return;
        };
        if path.is_dir() {
            hint(ui, "Folder input: each file's details are shown in the preset's file list after scanning.");
            return;
        }
        if !path.is_file() {
            hint(ui, "The input file doesn't exist.");
            return;
        }
        if self.slot.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("probing…");
            });
            return;
        }
        let info = match &self.slot.result {
            Some(Ok(i)) => i.clone(),
            Some(Err(e)) => {
                ui.colored_label(Color32::LIGHT_RED, format!("ffprobe error: {e}"));
                return;
            }
            None => return,
        };
        let s = summarize(&path, &info);
        egui::ScrollArea::vertical().id_salt("source_scroll").auto_shrink([false, false]).show(ui, |ui| {
            kv(ui, "src_g", &s.general);
            if !s.video.is_empty() {
                head(ui, "Video");
                kv(ui, "src_v", &s.video);
            }
            head(ui, &format!("Audio tracks ({})", s.audio.len()));
            list(ui, &s.audio, "none");
            head(ui, &format!("Subtitle tracks ({})", s.subtitles.len()));
            list(ui, &s.subtitles, "none");
            head(ui, &format!("Chapters ({})", s.chapters.len()));
            if s.chapters.is_empty() {
                ui.label(RichText::new("No chapters – use the chapter creator in Re-encode / Custom presets or the \"Add chapters\" preset.").small().color(Color32::from_rgb(230, 180, 60)));
            } else {
                egui::CollapsingHeader::new(format!("show {} chapter(s)", s.chapters.len())).id_salt("src_ch").default_open(s.chapters.len() <= 12).show(ui, |ui| {
                    for c in &s.chapters {
                        ui.label(RichText::new(c).small().monospace());
                    }
                });
            }
            if s.attachments > 0 {
                head(ui, "Attachments");
                ui.label(format!("{} (fonts, cover art…)", s.attachments));
            }
        });
    }
}

fn head(ui: &mut Ui, t: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(t).strong());
}

fn kv(ui: &mut Ui, id: &str, rows: &[(String, String)]) {
    egui::Grid::new(id).num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
        for (k, v) in rows {
            ui.label(RichText::new(k).weak());
            ui.label(v);
            ui.end_row();
        }
    });
}

fn list(ui: &mut Ui, rows: &[String], empty: &str) {
    if rows.is_empty() {
        ui.label(RichText::new(empty).weak());
    }
    for r in rows {
        ui.label(RichText::new(r).small());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffenc::ffmpeg::probe::parse_probe_json;

    #[test]
    fn summary_like_handbrake() {
        let j = r#"{"streams":[
            {"index":0,"codec_type":"video","codec_name":"h264","profile":"High","width":1920,"height":1080,"pix_fmt":"yuv420p","avg_frame_rate":"30000/1001","field_order":"tt"},
            {"index":1,"codec_type":"audio","codec_name":"aac","channel_layout":"stereo","sample_rate":"48000","bit_rate":"192000","tags":{"language":"eng","title":"Game"},"disposition":{"default":1}},
            {"index":2,"codec_type":"audio","codec_name":"opus","channels":1,"sample_rate":"44100","tags":{"language":"und"}},
            {"index":3,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"ger"},"disposition":{"forced":1}},
            {"index":4,"codec_type":"attachment","codec_name":"ttf"}],
          "chapters":[{"start_time":"0","end_time":"60","tags":{"title":"Intro"}},{"start_time":"60.5","end_time":"120"}],
          "format":{"duration":"120.0","format_long_name":"Matroska / WebM","size":"10485760","bit_rate":"699050"}}"#;
        let s = summarize(Path::new("/v/My Game.mkv"), &parse_probe_json(j).unwrap());
        let g: Vec<String> = s.general.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(g, ["File=My Game.mkv", "Container=Matroska / WebM", "Duration=00:02:00", "Size=10.0 MiB", "Bitrate=699 kb/s"]);
        let v: Vec<String> = s.video.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(v, ["Codec=h264 (High)", "Resolution=1920 x 1080", "Frame rate=29.97 fps", "Pixel format=yuv420p", "Scan=interlaced, top field first (tt)"]);
        assert_eq!(s.audio, ["1. aac, stereo, 48 kHz, 192 kb/s [eng] \"Game\" (default)", "2. opus, 1 ch, 44.1 kHz"]);
        assert_eq!(s.subtitles, ["1. subrip [ger] (forced)"]);
        assert_eq!(s.chapters, ["00:00:00  Intro", "00:01:00.500  Chapter 2"]);
        assert_eq!(s.attachments, 1);
        assert_eq!(interlace_text(Some("progressive")), "progressive");
        assert_eq!(interlace_text(None), "unknown (not reported)");
        assert_eq!(fmt_size(2_147_483_648), "2.00 GiB");
        assert_eq!(fmt_rate(8_500_000), "8.5 Mb/s");
    }
}
