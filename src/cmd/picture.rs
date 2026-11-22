//! Crop and watermark (1.1). Shared by every preset that re-encodes video.
//!
//! The settings ([`CropSettings`], [`WatermarkSettings`]) are stored in presets. Before a command
//! is built they are resolved against each file's real size into pixel values ([`CropPx`],
//! [`OverlayPx`]), so one preset works for 1080p and 4K files alike. With both options off the
//! builders get `PictureOps::default()` and emit exactly the commands they always did.
//!
//! Filter order inside the video chain: `crop` first, then deinterlace, then the watermark
//! overlay (so the watermark is placed on the cropped frame, like the preview shows), then
//! resize / range conversion / output frame rate.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::util::fmt_num;

// ---------------------------------------------------------------- crop

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CropSettings {
    pub enabled: bool,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Gentle note, never blocks.
    Note,
    /// Shown in red with a fix-it button; the job still runs.
    Warn,
    /// Impossible (nothing left of the picture): blocks the run.
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub text: String,
}

impl Issue {
    fn new(severity: Severity, text: impl Into<String>) -> Self {
        Issue { severity, text: text.into() }
    }
}

/// Pixel crop for one file: `crop=w:h:x:y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropPx {
    pub w: u32,
    pub h: u32,
    pub x: u32,
    pub y: u32,
}

impl CropPx {
    pub fn filter(&self) -> String {
        format!("crop={}:{}:{}:{}", self.w, self.h, self.x, self.y)
    }
}

impl CropSettings {
    /// Cropping is on and removes at least one pixel.
    pub fn active(&self) -> bool {
        self.enabled && (self.top | self.right | self.bottom | self.left) != 0
    }

    /// Size left after cropping a `w`×`h` frame (None = nothing left).
    pub fn result_size(&self, w: u32, h: u32) -> Option<(u32, u32)> {
        let cw = w.checked_sub(self.left.checked_add(self.right)?)?;
        let ch = h.checked_sub(self.top.checked_add(self.bottom)?)?;
        (cw > 0 && ch > 0).then_some((cw, ch))
    }

    /// Plain-language checks for a `w`×`h` source. Empty when cropping is off.
    pub fn check(&self, w: u32, h: u32) -> Vec<Issue> {
        let mut v = vec![];
        if !self.active() || w == 0 || h == 0 {
            return v;
        }
        let lr = self.left as u64 + self.right as u64;
        let tb = self.top as u64 + self.bottom as u64;
        if lr >= w as u64 {
            v.push(Issue::new(Severity::Error, format!("Left + Right ({lr} px) removes the whole width ({w} px).")));
        }
        if tb >= h as u64 {
            v.push(Issue::new(Severity::Error, format!("Top + Bottom ({tb} px) removes the whole height ({h} px).")));
        }
        if !v.is_empty() {
            return v;
        }
        // Top / Left move the picture, so they must be even. Right / Bottom only matter for the
        // final size (an odd-sized source needs one odd value there to end up even).
        let odd: Vec<String> = [("Top", self.top, true), ("Right", self.right, w % 2 == 0), ("Bottom", self.bottom, h % 2 == 0), ("Left", self.left, true)]
            .iter()
            .filter(|(_, n, matters)| *matters && n % 2 == 1)
            .map(|(s, n, _)| (s, n))
            .map(|(s, n)| format!("{s} {n}"))
            .collect();
        if !odd.is_empty() {
            v.push(Issue::new(Severity::Warn, format!("Odd crop value ({}). Normal video needs even numbers, or the encode may fail or be shifted.", odd.join(", "))));
        }
        let (cw, ch) = self.result_size(w, h).unwrap_or((0, 0));
        if cw % 2 == 1 || ch % 2 == 1 {
            v.push(Issue::new(Severity::Warn, format!("The result, {cw}×{ch}, has an odd size. Normal video needs an even width and height.")));
        } else if cw % 8 != 0 || ch % 8 != 0 {
            v.push(Issue::new(Severity::Note, format!("Note: {cw}×{ch} is not a multiple of 8. That's fine; multiples of 8 or 16 compress slightly better.")));
        }
        v
    }

    /// "Round to even": every odd value goes down to the next even number; if the source size
    /// itself is odd, one extra pixel is cropped (right / bottom) so the result is even.
    pub fn round_to_even(&mut self, w: u32, h: u32) {
        for n in [&mut self.top, &mut self.right, &mut self.bottom, &mut self.left] {
            *n -= *n % 2;
        }
        if let Some((cw, ch)) = self.result_size(w, h) {
            if cw % 2 == 1 {
                if cw > 1 {
                    self.right += 1;
                } else {
                    self.right = self.right.saturating_sub(1);
                }
            }
            if ch % 2 == 1 {
                if ch > 1 {
                    self.bottom += 1;
                } else {
                    self.bottom = self.bottom.saturating_sub(1);
                }
            }
        }
    }

    /// Pixel crop for a `w`×`h` file. Ok(None) when cropping is off; Err when nothing is left.
    pub fn resolve(&self, w: u32, h: u32) -> Result<Option<CropPx>, String> {
        if !self.active() {
            return Ok(None);
        }
        if w == 0 || h == 0 {
            return Err("Cropping needs the video size, but it is unknown.".into());
        }
        if let Some(e) = self.check(w, h).into_iter().find(|i| i.severity == Severity::Error) {
            return Err(format!("Crop: {}", e.text));
        }
        let (cw, ch) = self.result_size(w, h).ok_or("Crop removes the whole picture.")?;
        Ok(Some(CropPx { w: cw, h: ch, x: self.left, y: self.top }))
    }
}

// ---------------------------------------------------------------- watermark

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Snap {
    TopLeft,
    TopRight,
    Center,
    BottomLeft,
    BottomRight,
}

impl Snap {
    pub const ALL: [Snap; 5] = [Snap::TopLeft, Snap::TopRight, Snap::Center, Snap::BottomLeft, Snap::BottomRight];
    pub fn label(self) -> &'static str {
        match self {
            Snap::TopLeft => "Top left",
            Snap::TopRight => "Top right",
            Snap::Center => "Center",
            Snap::BottomLeft => "Bottom left",
            Snap::BottomRight => "Bottom right",
        }
    }
}

/// Watermark placement, stored as fractions of the (cropped) frame so it scales across sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatermarkSettings {
    pub enabled: bool,
    /// Image file (anything FFmpeg can read; PNG transparency is kept).
    pub image: String,
    /// Left edge, fraction of the frame width.
    pub x: f64,
    /// Top edge, fraction of the frame height.
    pub y: f64,
    /// Width, fraction of the frame width.
    pub w: f64,
    /// Height, fraction of the frame height (only used when the aspect ratio is unlocked).
    pub h: f64,
    /// Keep the image's own shape (default on).
    pub lock_aspect: bool,
    /// 0–100 %.
    pub opacity_pct: u32,
    /// Gap kept from the edges by the snap buttons, % of the shorter frame side.
    pub margin_pct: f64,
}

impl Default for WatermarkSettings {
    fn default() -> Self {
        WatermarkSettings { enabled: false, image: String::new(), x: 0.77, y: 0.04, w: 0.2, h: 0.1, lock_aspect: true, opacity_pct: 70, margin_pct: 3.0 }
    }
}

/// A watermark resolved for one file.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayPx {
    pub image: PathBuf,
    pub w: u32,
    /// None = keep the image's aspect ratio (`scale=W:-1`).
    pub h: Option<u32>,
    pub x: i64,
    pub y: i64,
    /// 0.0–1.0
    pub opacity: f64,
}

impl OverlayPx {
    /// `[N:v]scale=W:H,format=rgba[,colorchannelmixer=aa=O][wm]` for input `N`.
    pub fn image_chain(&self, input: usize) -> String {
        let h = self.h.map(|h| h.to_string()).unwrap_or_else(|| "-1".into());
        let mut s = format!("[{input}:v]scale={}:{h},format=rgba", self.w);
        if self.opacity < 1.0 {
            s.push_str(&format!(",colorchannelmixer=aa={}", fmt_num((self.opacity.max(0.0) * 1000.0).round() / 1000.0)));
        }
        s.push_str("[wm]");
        s
    }
    /// The overlay filter (a still image is held for the whole video; output length = the video).
    pub fn overlay_filter(&self) -> String {
        format!("overlay=x={}:y={}:format=auto:eof_action=repeat", self.x, self.y)
    }
}

impl WatermarkSettings {
    pub fn opacity(&self) -> f64 {
        self.opacity_pct.min(100) as f64 / 100.0
    }

    /// Placed size in frame pixels (fw×fh frame, image iw×ih when known).
    pub fn size_px(&self, fw: f64, fh: f64, img: Option<(u32, u32)>) -> (f64, f64) {
        let w = (self.w * fw).max(1.0);
        let h = match (self.lock_aspect, img) {
            (true, Some((iw, ih))) if iw > 0 && ih > 0 => w * ih as f64 / iw as f64,
            _ => (self.h * fh).max(1.0),
        };
        (w, h)
    }

    /// Move to a corner / the center, keeping the margin.
    pub fn snap(&mut self, s: Snap, fw: f64, fh: f64, img: Option<(u32, u32)>) {
        if fw <= 0.0 || fh <= 0.0 {
            return;
        }
        let (w, h) = self.size_px(fw, fh, img);
        let m = self.margin_pct.max(0.0) / 100.0 * fw.min(fh);
        let (x, y) = match s {
            Snap::TopLeft => (m, m),
            Snap::TopRight => (fw - w - m, m),
            Snap::Center => ((fw - w) / 2.0, (fh - h) / 2.0),
            Snap::BottomLeft => (m, fh - h - m),
            Snap::BottomRight => (fw - w - m, fh - h - m),
        };
        self.x = (x / fw).max(0.0);
        self.y = (y / fh).max(0.0);
    }

    /// Pixel placement on a `fw`×`fh` frame (the cropped size). Ok(None) when off.
    /// The watermark is kept inside the frame (moved in when a differently shaped file
    /// would push it over an edge).
    pub fn resolve(&self, fw: u32, fh: u32, img: Option<(u32, u32)>) -> Result<(Option<OverlayPx>, Vec<String>), String> {
        if !self.enabled {
            return Ok((None, vec![]));
        }
        let image = self.image.trim();
        if image.is_empty() {
            return Err("Add watermark is ticked, but no watermark image is chosen. Use “Set up watermark…”.".into());
        }
        if fw == 0 || fh == 0 {
            return Err("The watermark needs the video size, but it is unknown.".into());
        }
        let mut warnings = vec![];
        let (w, h) = self.size_px(fw as f64, fh as f64, img);
        let wp = (w.round() as u32).max(1);
        let hp = (h.round() as u32).max(1);
        if wp > fw || hp > fh {
            warnings.push(format!("The watermark ({wp}×{hp}) is bigger than the picture ({fw}×{fh})."));
        }
        let max_x = (fw as i64 - wp as i64).max(0);
        let max_y = (fh as i64 - hp as i64).max(0);
        let x = ((self.x * fw as f64).round() as i64).clamp(0, max_x);
        let y = ((self.y * fh as f64).round() as i64).clamp(0, max_y);
        if self.opacity_pct == 0 {
            warnings.push("Watermark opacity is 0 %, so it will be invisible.".into());
        }
        let keep_aspect = self.lock_aspect;
        Ok((
            Some(OverlayPx { image: PathBuf::from(image), w: wp, h: (!keep_aspect).then_some(hp), x, y, opacity: self.opacity() }),
            warnings,
        ))
    }
}

// ---------------------------------------------------------------- both

/// Crop + watermark as saved in a preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PictureSettings {
    pub crop: CropSettings,
    pub watermark: WatermarkSettings,
}

impl PictureSettings {
    pub fn any(&self) -> bool {
        self.crop.active() || self.watermark.enabled
    }
    /// Resolve for a `w`×`h` file with a watermark image of `img` pixels (when known).
    /// Returns the ops plus warnings for that file.
    pub fn resolve(&self, w: u32, h: u32, img: Option<(u32, u32)>) -> Result<(PictureOps, Vec<String>), String> {
        let crop = self.crop.resolve(w, h)?;
        let (fw, fh) = crop.map(|c| (c.w, c.h)).unwrap_or((w, h));
        let (overlay, mut warnings) = self.watermark.resolve(fw, fh, img)?;
        if self.crop.active() {
            for i in self.crop.check(w, h) {
                if i.severity == Severity::Warn {
                    warnings.push(format!("Crop: {}", i.text));
                }
            }
        }
        Ok((PictureOps { crop, overlay }, warnings))
    }
}

/// Resolved per-file crop / watermark handed to the command builders.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PictureOps {
    pub crop: Option<CropPx>,
    pub overlay: Option<OverlayPx>,
}

impl PictureOps {
    pub fn crop_filter(&self) -> Option<String> {
        self.crop.map(|c| c.filter())
    }
    /// Frame size after the crop.
    pub fn cropped_size(&self, w: u32, h: u32) -> (u32, u32) {
        self.crop.map(|c| (c.w, c.h)).unwrap_or((w, h))
    }
}

/// `-i IMAGE` for the watermark (a single frame, held by the overlay).
pub fn image_input_args(ov: &OverlayPx) -> Vec<String> {
    vec!["-i".into(), super::p(&ov.image)]
}

/// One complex filter graph: `[in]pre[base];[N:v]…[wm];[base][wm]overlay…,post[out]`.
/// `input` / `out` are pad names without brackets (e.g. "0:v", "vout").
pub fn overlay_graph(input: &str, pre: Option<&str>, ov: &OverlayPx, wm_input: usize, post: Option<&str>, out: &str) -> String {
    let mut parts = vec![ov.image_chain(wm_input)];
    let base = match pre.filter(|s| !s.is_empty()) {
        Some(f) => {
            parts.push(format!("[{input}]{f}[base]"));
            "base".to_string()
        }
        None => input.to_string(),
    };
    let mut last = format!("[{base}][wm]{}", ov.overlay_filter());
    if let Some(f) = post.filter(|s| !s.is_empty()) {
        last.push(',');
        last.push_str(f);
    }
    last.push_str(&format!("[{out}]"));
    parts.push(last);
    parts.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crop(t: u32, r: u32, b: u32, l: u32) -> CropSettings {
        CropSettings { enabled: true, top: t, right: r, bottom: b, left: l }
    }

    #[test]
    fn crop_off_or_zero_is_inactive() {
        assert!(!CropSettings::default().active());
        assert!(!crop(0, 0, 0, 0).active());
        assert!(!CropSettings { enabled: false, top: 10, ..Default::default() }.active());
        assert_eq!(CropSettings { enabled: false, top: 10, ..Default::default() }.resolve(1920, 1080).unwrap(), None);
        assert!(crop(0, 0, 0, 0).check(1920, 1080).is_empty());
    }

    #[test]
    fn crop_result_and_filter() {
        let c = crop(140, 0, 140, 0);
        assert_eq!(c.result_size(1920, 1080), Some((1920, 800)));
        assert_eq!(c.resolve(1920, 1080).unwrap().unwrap().filter(), "crop=1920:800:0:140");
        let c = crop(10, 20, 30, 40);
        assert_eq!(c.resolve(1920, 1080).unwrap().unwrap(), CropPx { w: 1860, h: 1040, x: 40, y: 10 });
        // 1860x1040: even, but 1860 is not a multiple of 8 -> only a gentle note
        let i = c.check(1920, 1080);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].severity, Severity::Note);
    }

    #[test]
    fn crop_validation_levels() {
        // whole width / height removed = error that blocks
        let c = crop(0, 960, 0, 960);
        assert_eq!(c.check(1920, 1080)[0].severity, Severity::Error);
        assert!(c.resolve(1920, 1080).is_err());
        let c = crop(600, 0, 480, 0);
        assert_eq!(c.check(1920, 1080)[0].severity, Severity::Error);
        assert!(c.result_size(1920, 1080).is_none());
        // odd values = warning (still resolves)
        let c = crop(3, 0, 0, 0);
        let i = c.check(1920, 1080);
        assert!(i.iter().any(|i| i.severity == Severity::Warn && i.text.contains("Top 3")), "{i:?}");
        assert!(i.iter().any(|i| i.severity == Severity::Warn && i.text.contains("1920×1077")), "{i:?}");
        assert!(c.resolve(1920, 1080).unwrap().is_some());
        // not a multiple of 8 = note only
        let c = crop(2, 0, 0, 0);
        let i = c.check(1920, 1080);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].severity, Severity::Note);
        // multiple of 8 = nothing
        assert!(crop(140, 0, 140, 0).check(1920, 1080).is_empty());
        // odd source with even values = odd result warning
        let i = crop(2, 0, 0, 0).check(1919, 1080);
        assert!(i.iter().any(|i| i.severity == Severity::Warn), "{i:?}");
    }

    #[test]
    fn round_to_even_fixes_everything() {
        let mut c = crop(3, 5, 7, 9);
        c.round_to_even(1920, 1080);
        assert_eq!((c.top, c.right, c.bottom, c.left), (2, 4, 6, 8));
        assert!(c.check(1920, 1080).iter().all(|i| i.severity == Severity::Note));
        // odd source: one more pixel cropped on the right / bottom
        let mut c = crop(1, 0, 0, 0);
        c.round_to_even(1921, 1081);
        assert_eq!(c.result_size(1921, 1081), Some((1920, 1080)));
        assert!(c.check(1921, 1081).iter().all(|i| i.severity != Severity::Warn));
    }

    #[test]
    fn watermark_resolve_and_chain() {
        let mut w = WatermarkSettings { enabled: true, image: "/img/logo.png".into(), x: 0.75, y: 0.05, w: 0.2, opacity_pct: 50, ..Default::default() };
        let (ov, warn) = w.resolve(1920, 1080, Some((400, 200))).unwrap();
        let ov = ov.unwrap();
        assert!(warn.is_empty());
        assert_eq!((ov.w, ov.h, ov.x, ov.y), (384, None, 1440, 54));
        assert_eq!(ov.image_chain(1), "[1:v]scale=384:-1,format=rgba,colorchannelmixer=aa=0.5[wm]");
        assert_eq!(ov.overlay_filter(), "overlay=x=1440:y=54:format=auto:eof_action=repeat");
        // same settings on 4K: same place, twice the pixels
        let ov4 = w.resolve(3840, 2160, Some((400, 200))).unwrap().0.unwrap();
        assert_eq!((ov4.w, ov4.x, ov4.y), (768, 2880, 108));
        // full opacity: no colour mixer
        w.opacity_pct = 100;
        assert_eq!(w.resolve(1920, 1080, None).unwrap().0.unwrap().image_chain(2), "[2:v]scale=384:-1,format=rgba[wm]");
        // unlocked aspect: explicit height
        w.lock_aspect = false;
        w.h = 0.1;
        assert_eq!(w.resolve(1920, 1080, None).unwrap().0.unwrap().h, Some(108));
        // kept inside the frame
        w.x = 0.95;
        w.y = 0.99;
        let ov = w.resolve(1920, 1080, None).unwrap().0.unwrap();
        assert_eq!((ov.x, ov.y), (1920 - 384, 1080 - 108));
        // off / no image
        assert_eq!(WatermarkSettings::default().resolve(1920, 1080, None).unwrap().0, None);
        let none = WatermarkSettings { enabled: true, ..Default::default() };
        assert!(none.resolve(1920, 1080, None).is_err());
    }

    #[test]
    fn snap_positions_keep_margin() {
        let mut w = WatermarkSettings { w: 0.25, margin_pct: 5.0, ..Default::default() };
        let img = Some((200, 100)); // 2:1 -> 480x240 on 1920x1080
        w.snap(Snap::BottomRight, 1920.0, 1080.0, img);
        let m = 54.0;
        assert!((w.x * 1920.0 - (1920.0 - 480.0 - m)).abs() < 1e-6);
        assert!((w.y * 1080.0 - (1080.0 - 240.0 - m)).abs() < 1e-6);
        w.snap(Snap::TopLeft, 1920.0, 1080.0, img);
        assert!((w.x * 1920.0 - m).abs() < 1e-6 && (w.y * 1080.0 - m).abs() < 1e-6);
        w.snap(Snap::Center, 1920.0, 1080.0, img);
        assert!((w.x * 1920.0 - 720.0).abs() < 1e-6 && (w.y * 1080.0 - 420.0).abs() < 1e-6);
    }

    #[test]
    fn graph_strings() {
        let ov = OverlayPx { image: "/i/l.png".into(), w: 200, h: None, x: 10, y: 20, opacity: 0.35 };
        assert_eq!(
            overlay_graph("0:v", Some("crop=1920:800:0:140"), &ov, 1, Some("scale=1280:534"), "vout"),
            "[1:v]scale=200:-1,format=rgba,colorchannelmixer=aa=0.35[wm];[0:v]crop=1920:800:0:140[base];[base][wm]overlay=x=10:y=20:format=auto:eof_action=repeat,scale=1280:534[vout]"
        );
        assert_eq!(
            overlay_graph("0:v", None, &ov, 2, None, "vout"),
            "[2:v]scale=200:-1,format=rgba,colorchannelmixer=aa=0.35[wm];[0:v][wm]overlay=x=10:y=20:format=auto:eof_action=repeat[vout]"
        );
    }

    #[test]
    fn picture_resolve_uses_cropped_frame_for_watermark() {
        let p = PictureSettings {
            crop: crop(140, 0, 140, 0),
            watermark: WatermarkSettings { enabled: true, image: "/l.png".into(), x: 0.0, y: 0.5, w: 0.1, ..Default::default() },
        };
        let (ops, _) = p.resolve(1920, 1080, None).unwrap();
        assert_eq!(ops.crop_filter().as_deref(), Some("crop=1920:800:0:140"));
        assert_eq!(ops.overlay.unwrap().y, 400);
        assert_eq!(PictureSettings::default().resolve(1920, 1080, None).unwrap().0, PictureOps::default());
    }

    #[test]
    fn old_presets_load_with_everything_off() {
        let p: PictureSettings = serde_json::from_str("{}").unwrap();
        assert!(!p.any());
        let w: WatermarkSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(w.lock_aspect && w.opacity_pct == 70);
    }
}
