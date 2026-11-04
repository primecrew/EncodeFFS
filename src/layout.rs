//! Window layout persistence (0.6.0-dev.2): window position/size/maximized state, panel sizes and
//! open/closed sections, kept in their own small file next to the settings file, so a broken or
//! old layout file can never stop the app from starting (anything unreadable = defaults).
//!
//! Units: the window position is in physical pixels of the virtual desktop (what Windows uses, so
//! it also says which monitor the window was on); the window size is in logical pixels at 100 %
//! text size (physical / monitor DPI scale), so it looks the same on a monitor with another scale.
//! Panel sizes are egui points (they follow the app's text size).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Layout file next to the portable settings file.
pub const PORTABLE_FILE: &str = "FFmpegEncoder-Experimental.layout.json";
/// Layout file in the %APPDATA% folder (installed copies).
pub const ROAMING_FILE: &str = "layout.json";
pub const FORMAT_VERSION: u32 = 1;

/// Window size on first start / after "Reset window layout" (logical pixels).
pub const DEFAULT_SIZE: [f32; 2] = [1380.0, 900.0];
/// The window's minimum size (also the smallest size ever restored).
pub const MIN_SIZE: [f32; 2] = [820.0, 600.0];
/// Largest size accepted from the file (anything bigger is garbage).
const MAX_SIDE: f32 = 32_768.0;
/// A restored window must show at least this much of its title bar on a connected screen
/// (physical pixels), otherwise it is treated as off-screen.
const MIN_VISIBLE_W: i32 = 160;
const TITLE_BAR_H: i32 = 32;
/// Room for the title bar and borders around the inner size (logical px; Windows 10/11 is about
/// 16 x 39), so a restored window fits on the screen including its frame.
pub const FRAME: [f32; 2] = [16.0, 40.0];

/// Integer rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RectPx {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RectPx {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        RectPx { x, y, w, h }
    }
    fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }
    fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }
    pub fn intersection_area(&self, o: &RectPx) -> i64 {
        let w = (self.right().min(o.right()) - (self.x.max(o.x)) as i64).max(0);
        let h = (self.bottom().min(o.bottom()) - (self.y.max(o.y)) as i64).max(0);
        w * h
    }
    fn intersection_w_h(&self, o: &RectPx) -> (i64, i64) {
        ((self.right().min(o.right()) - (self.x.max(o.x)) as i64).max(0), (self.bottom().min(o.bottom()) - (self.y.max(o.y)) as i64).max(0))
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && (x as i64) < self.right() && y >= self.y && (y as i64) < self.bottom()
    }
}

/// A connected screen: its work area (without the taskbar) and DPI scale (1.0 = 96 dpi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub work: RectPx,
    pub scale: f32,
    pub primary: bool,
}

/// Saved window geometry. Position/size are the normal (not maximized) window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeom {
    /// Outer top-left corner, physical pixels.
    pub x: i32,
    pub y: i32,
    /// Inner (client) size, logical pixels at 100 % text size.
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
    /// Work area of the monitor the window was on when saved (informational / same-monitor check).
    pub monitor: Option<RectPx>,
    /// That monitor's DPI scale when saved.
    pub scale: f32,
}

impl Default for WindowGeom {
    fn default() -> Self {
        WindowGeom { x: 0, y: 0, w: DEFAULT_SIZE[0], h: DEFAULT_SIZE[1], maximized: false, monitor: None, scale: 1.0 }
    }
}

impl WindowGeom {
    /// Sizes that make sense at all (finite, not tiny, not absurd).
    pub fn is_sane(&self) -> bool {
        let ok = |v: f32| v.is_finite() && v >= 100.0 && v <= MAX_SIDE;
        ok(self.w) && ok(self.h) && self.scale.is_finite() && self.scale > 0.1 && self.scale < 10.0 && self.x.unsigned_abs() < 1_000_000 && self.y.unsigned_abs() < 1_000_000
    }
}

/// Sizes of the resizable panels (egui points). `None` = the built-in default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Panels {
    /// Left preset list.
    pub presets_width: Option<f32>,
    /// Right "Source" info panel.
    pub source_width: Option<f32>,
    /// Bottom queue / log panel.
    pub queue_height: Option<f32>,
}

fn sane_panel(v: Option<f32>) -> Option<f32> {
    v.filter(|v| v.is_finite() && (40.0..=8000.0).contains(v))
}

impl Panels {
    pub fn sanitized(self) -> Panels {
        Panels { presets_width: sane_panel(self.presets_width), source_width: sane_panel(self.source_width), queue_height: sane_panel(self.queue_height) }
    }
}

/// Size (and, for dialogs that can be moved, position) of a resizable dialog, egui points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DialogRect {
    pub w: f32,
    pub h: f32,
    /// Top-left corner; `None` for dialogs that are always centered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
}

impl DialogRect {
    pub fn is_sane(&self) -> bool {
        let side = |v: f32| v.is_finite() && (60.0..=16_000.0).contains(&v);
        side(self.w) && side(self.h) && self.pos.map_or(true, |[x, y]| x.is_finite() && y.is_finite() && x.abs() < 100_000.0 && y.abs() < 100_000.0)
    }
}

/// Everything saved in the layout file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    pub window: Option<WindowGeom>,
    pub panels: Panels,
    /// Open (true) / closed (false) state of collapsible sections, by a stable key
    /// (e.g. "settings.compressor_advanced", "presets.group:Video").
    pub sections: BTreeMap<String, bool>,
    /// Resizable dialogs by key ("settings", "confirm", "chapter_creator", "updates", "about", "help"),
    /// 0.6.0-dev.3; older files have none.
    #[serde(default)]
    pub dialogs: BTreeMap<String, DialogRect>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { version: FORMAT_VERSION, window: None, panels: Panels::default(), sections: BTreeMap::new(), dialogs: BTreeMap::new() }
    }
}

impl Layout {
    /// Lenient parse: each part that can't be read falls back to its default on its own;
    /// text that isn't JSON at all gives the default layout. Never fails.
    pub fn from_json(text: &str) -> Layout {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Layout::default() };
        if !v.is_object() {
            return Layout::default();
        }
        let window = v.get("window").and_then(|w| serde_json::from_value::<WindowGeom>(w.clone()).ok()).filter(|g| g.is_sane());
        let panels = v.get("panels").and_then(|p| serde_json::from_value::<Panels>(p.clone()).ok()).unwrap_or_default().sanitized();
        let mut sections = BTreeMap::new();
        if let Some(obj) = v.get("sections").and_then(|s| s.as_object()) {
            for (k, val) in obj {
                if let Some(b) = val.as_bool() {
                    sections.insert(k.clone(), b);
                }
            }
        }
        let mut dialogs = BTreeMap::new();
        if let Some(obj) = v.get("dialogs").and_then(|s| s.as_object()) {
            for (k, val) in obj {
                if let Ok(d) = serde_json::from_value::<DialogRect>(val.clone()) {
                    if d.is_sane() {
                        dialogs.insert(k.clone(), d);
                    }
                }
            }
        }
        Layout { version: FORMAT_VERSION, window, panels, sections, dialogs }
    }

    pub fn load_from(path: &Path) -> Layout {
        std::fs::read_to_string(path).map(|t| Layout::from_json(&t)).unwrap_or_default()
    }

    /// Write-then-rename, so a crash never leaves a half-written file.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("Could not save window layout to {}: {e}", path.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("Could not save window layout to {}: {e}", path.display()))
    }
}

/// The layout file that belongs to a settings file (same folder; portable or %APPDATA% name).
pub fn layout_path_for(settings_path: &Path) -> PathBuf {
    let dir = settings_path.parent().map(Path::to_path_buf).unwrap_or_default();
    if settings_path.file_name().and_then(|n| n.to_str()) == Some(crate::settings::PORTABLE_FILE) {
        dir.join(PORTABLE_FILE)
    } else {
        dir.join(ROAMING_FILE)
    }
}

pub fn layout_path() -> PathBuf {
    layout_path_for(&crate::settings::settings_path())
}

/// Delete the saved layout (Reset window layout / option turned off). Missing file is fine.
pub fn forget(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Could not delete {}: {e}", path.display())),
    }
}

/// Where and how big to open the window.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Outer top-left in physical pixels; `None` = centered by the system/app.
    pub pos: Option<(i32, i32)>,
    /// Inner size, logical pixels at 100 %.
    pub size: [f32; 2],
    pub maximized: bool,
    /// DPI scale of the monitor it opens on (to turn `pos` into egui points).
    pub scale: f32,
    /// True when a saved layout was used.
    pub restored: bool,
}

fn primary(monitors: &[Monitor]) -> Option<&Monitor> {
    monitors.iter().find(|m| m.primary).or_else(|| monitors.first())
}

/// Clamp a logical inner size to [MIN_SIZE, monitor work area minus the window frame].
fn clamp_size(size: [f32; 2], m: Option<&Monitor>) -> [f32; 2] {
    let (max_w, max_h) = match m {
        Some(m) => ((m.work.w as f32 / m.scale - FRAME[0]).max(200.0), (m.work.h as f32 / m.scale - FRAME[1]).max(200.0)),
        None => (MAX_SIDE, MAX_SIDE),
    };
    let fit = |v: f32, min: f32, max: f32| if v.is_finite() { v.clamp(min.min(max), max) } else { min.min(max) };
    [fit(size[0], MIN_SIZE[0], max_w), fit(size[1], MIN_SIZE[1], max_h)]
}

/// The default: default size (shrunk to fit the primary screen), centered.
pub fn default_placement(monitors: &[Monitor]) -> Placement {
    let m = primary(monitors);
    Placement { pos: None, size: clamp_size(DEFAULT_SIZE, m), maximized: false, scale: m.map_or(1.0, |m| m.scale), restored: false }
}

/// Centered position (physical) for a window of `size` (logical) on monitor `m`.
pub fn centered_on(m: &Monitor, size: [f32; 2]) -> (i32, i32) {
    let (w, h) = (((size[0] + FRAME[0]) * m.scale) as i32, ((size[1] + FRAME[1]) * m.scale) as i32);
    (m.work.x + (m.work.w - w).max(0) / 2, m.work.y + (m.work.h - h).max(0) / 2)
}

/// Decide the startup window placement.
/// * option off, nothing saved, or unusable values -> default (centered, default size)
/// * saved window whose title bar isn't visibly on any connected screen -> default
/// * otherwise the saved place on the monitor it overlaps most, size clamped to that monitor and
///   to the minimum size, position moved so the whole window is on that monitor.
///
/// `monitors` empty = the screens can't be listed (non-Windows): size and maximized state are
/// restored, the window is centered.
pub fn placement(remember: bool, saved: Option<&WindowGeom>, monitors: &[Monitor]) -> Placement {
    let Some(g) = saved.filter(|g| remember && g.is_sane()) else { return default_placement(monitors) };
    if monitors.is_empty() {
        return Placement { pos: None, size: clamp_size([g.w, g.h], None), maximized: g.maximized, scale: 1.0, restored: true };
    }
    // the saved title bar strip, in physical pixels (size at the saved monitor's scale)
    let outer_w = (g.w * g.scale).round() as i32;
    let title = RectPx::new(g.x, g.y, outer_w.max(1), TITLE_BAR_H);
    let best = monitors
        .iter()
        .map(|m| (m, m.work.intersection_w_h(&title)))
        .filter(|(_, (w, h))| *w >= MIN_VISIBLE_W.min(outer_w) as i64 && *h >= (TITLE_BAR_H / 2) as i64)
        .max_by_key(|(_, (w, h))| w * h)
        .map(|(m, _)| m);
    let Some(m) = best else { return default_placement(monitors) };
    let size = clamp_size([g.w, g.h], Some(m));
    // outer size (with frame), physical
    let (pw, ph) = (((size[0] + FRAME[0]) * m.scale).round() as i32, ((size[1] + FRAME[1]) * m.scale).round() as i32);
    // keep the whole window on this monitor's work area
    let x = g.x.min(m.work.x + m.work.w - pw).max(m.work.x);
    let y = g.y.min(m.work.y + m.work.h - ph).max(m.work.y);
    Placement { pos: Some((x, y)), size, maximized: g.maximized, scale: m.scale, restored: true }
}

/// The monitor a physical point is on (or the one nearest to it).
pub fn monitor_at(monitors: &[Monitor], x: i32, y: i32) -> Option<&Monitor> {
    monitors.iter().find(|m| m.work.contains(x, y)).or_else(|| {
        monitors.iter().min_by_key(|m| {
            let cx = m.work.x as i64 + m.work.w as i64 / 2;
            let cy = m.work.y as i64 + m.work.h as i64 / 2;
            (cx - x as i64).pow(2) + (cy - y as i64).pow(2)
        })
    })
}

/// Connected screens (Windows: work areas in physical pixels; the app is per-monitor DPI aware
/// through its manifest). Empty elsewhere or when they can't be read.
#[cfg(windows)]
pub fn monitors() -> Vec<Monitor> {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    const MONITORINFOF_PRIMARY: u32 = 1;
    unsafe extern "system" fn cb(hm: HMONITOR, _dc: HDC, _r: *mut RECT, data: LPARAM) -> BOOL {
        let out = &mut *(data as *mut Vec<Monitor>);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hm, &mut mi) != 0 {
            let r = mi.rcWork;
            let (mut dx, mut dy) = (96u32, 96u32);
            let scale = if GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) == 0 && dx > 0 { dx as f32 / 96.0 } else { 1.0 };
            out.push(Monitor { work: RectPx::new(r.left, r.top, r.right - r.left, r.bottom - r.top), scale, primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0 });
        }
        1
    }
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(cb), &mut out as *mut Vec<Monitor> as LPARAM);
    }
    out.retain(|m| m.work.w > 0 && m.work.h > 0);
    out
}

#[cfg(not(windows))]
pub fn monitors() -> Vec<Monitor> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i32, y: i32, w: i32, h: i32, scale: f32, primary: bool) -> Monitor {
        Monitor { work: RectPx::new(x, y, w, h), scale, primary }
    }
    fn two_screens() -> Vec<Monitor> {
        vec![mon(0, 0, 1920, 1040, 1.0, true), mon(1920, 0, 2560, 1400, 1.5, false)]
    }
    fn geom(x: i32, y: i32, w: f32, h: f32) -> WindowGeom {
        WindowGeom { x, y, w, h, maximized: false, monitor: None, scale: 1.0 }
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("ffenc_lay_{}", crate::util::unique_id()));
        let p = dir.join(PORTABLE_FILE);
        let mut l = Layout::default();
        l.window = Some(WindowGeom { x: 2000, y: 40, w: 1200.0, h: 800.0, maximized: true, monitor: Some(RectPx::new(1920, 0, 2560, 1400)), scale: 1.5 });
        l.panels = Panels { presets_width: Some(310.5), source_width: Some(260.0), queue_height: Some(180.0) };
        l.sections.insert("settings.compressor_advanced".into(), true);
        l.sections.insert("presets.group:Video".into(), false);
        l.dialogs.insert("settings".into(), DialogRect { w: 700.0, h: 540.0, pos: Some([120.0, 80.0]) });
        l.dialogs.insert("confirm".into(), DialogRect { w: 950.0, h: 600.0, pos: None });
        l.save_to(&p).unwrap();
        assert_eq!(Layout::load_from(&p), l);
        assert!(!dir.join(format!("{PORTABLE_FILE}.tmp")).exists());
        forget(&p).unwrap();
        assert!(!p.exists());
        forget(&p).unwrap(); // missing is fine
        assert_eq!(Layout::load_from(&p), Layout::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_missing_or_broken_files_give_defaults() {
        // no file / empty / not JSON / wrong type
        for t in ["", "garbage{", "[1,2]", "null", "{}"] {
            assert_eq!(Layout::from_json(t), Layout::default(), "{t:?}");
        }
        // missing fields: what's there is used, the rest default
        let l = Layout::from_json(r#"{"panels": {"presets_width": 300}}"#);
        assert_eq!((l.window.clone(), l.panels.presets_width, l.panels.queue_height), (None, Some(300.0), None));
        let l = Layout::from_json(r#"{"window": {"x": 10, "y": 20}}"#);
        assert_eq!(l.window.map(|w| (w.x, w.y, w.w, w.h, w.maximized)), Some((10, 20, DEFAULT_SIZE[0], DEFAULT_SIZE[1], false)));
        // one broken part doesn't take the others with it
        let l = Layout::from_json(r#"{"window": "big", "panels": {"queue_height": 150, "source_width": "wide"}, "sections": {"a": true, "b": 3}}"#);
        assert_eq!(l.window, None);
        assert_eq!(l.panels, Panels::default(), "a panels object with a bad field is ignored as a whole");
        assert_eq!(l.sections.into_iter().collect::<Vec<_>>(), vec![("a".to_string(), true)]);
        // dialogs: the good ones survive, broken/tiny ones are dropped; a dev.2 file has none
        let l = Layout::from_json(r#"{"dialogs": {"settings": {"w": 700, "h": 500, "pos": [10, 20]}, "about": {"w": 5, "h": 500}, "help": "x", "confirm": {"w": 900, "h": 500}}}"#);
        assert_eq!(l.dialogs.keys().cloned().collect::<Vec<_>>(), vec!["confirm".to_string(), "settings".to_string()]);
        assert_eq!(l.dialogs["settings"].pos, Some([10.0, 20.0]));
        assert!(Layout::from_json(r#"{"window": {"x": 1, "y": 2, "w": 900, "h": 700}, "panels": {"presets_width": 300}}"#).dialogs.is_empty());
        // zero / tiny / absurd / NaN-ish values are dropped
        assert_eq!(Layout::from_json(r#"{"window": {"x": 0, "y": 0, "w": 0, "h": 0}}"#).window, None);
        assert_eq!(Layout::from_json(r#"{"window": {"w": 50, "h": 900}}"#).window, None);
        assert_eq!(Layout::from_json(r#"{"window": {"w": 1e9, "h": 900}}"#).window, None);
        assert_eq!(Layout::from_json(r#"{"window": {"w": 1000, "h": 900, "scale": 0}}"#).window, None);
        assert_eq!(Layout::from_json(r#"{"panels": {"presets_width": -5, "queue_height": 1e30}}"#).panels, Panels::default());
        // settings files from before this feature have no layout file at all -> defaults
        let s = crate::settings::Settings::from_json(r#"{"crf": 20}"#).unwrap();
        assert!(s.remember_layout, "on by default for older settings files");
    }

    #[test]
    fn layout_file_sits_next_to_the_settings_file() {
        let portable = Path::new("/x/app").join(crate::settings::PORTABLE_FILE);
        assert_eq!(layout_path_for(&portable), Path::new("/x/app").join(PORTABLE_FILE));
        assert_eq!(layout_path_for(Path::new("/r/FFmpegEncoder-Experimental/settings.json")), Path::new("/r/FFmpegEncoder-Experimental/layout.json"));
        assert!(PORTABLE_FILE.starts_with(crate::FILE_STEM));
    }

    #[test]
    fn valid_saved_place_is_restored() {
        let ms = two_screens();
        let p = placement(true, Some(&geom(100, 80, 1200.0, 800.0)), &ms);
        assert_eq!(p, Placement { pos: Some((100, 80)), size: [1200.0, 800.0], maximized: false, scale: 1.0, restored: true });
        // on the second (150 %) monitor
        let g = WindowGeom { x: 2100, y: 50, w: 1300.0, h: 850.0, maximized: true, monitor: Some(ms[1].work), scale: 1.5 };
        let p = placement(true, Some(&g), &ms);
        assert_eq!((p.pos, p.size, p.maximized, p.scale), (Some((2100, 50)), [1300.0, 850.0], true, 1.5));
    }

    #[test]
    fn off_screen_or_disconnected_falls_back_to_centered_default() {
        let ms = two_screens();
        let d = default_placement(&ms);
        assert_eq!((d.pos, d.size, d.maximized, d.restored), (None, DEFAULT_SIZE, false, false));
        // was on a third monitor that's gone
        assert_eq!(placement(true, Some(&geom(-2500, 100, 1200.0, 800.0)), &ms), d);
        // far away / below all screens
        assert_eq!(placement(true, Some(&geom(10_000, 10_000, 1200.0, 800.0)), &ms), d);
        assert_eq!(placement(true, Some(&geom(100, 1200, 1200.0, 800.0)), &ms), d);
        // title bar above the top edge (can't be grabbed)
        assert_eq!(placement(true, Some(&geom(100, -400, 1200.0, 800.0)), &ms), d);
        // only a sliver of the title bar visible
        assert_eq!(placement(true, Some(&geom(1900 + 2560, 100, 1200.0, 800.0)), &ms), d);
        // the second monitor was unplugged: a window that was there falls back
        let one = vec![ms[0]];
        assert_eq!(placement(true, Some(&geom(2100, 100, 1200.0, 800.0)), &one), default_placement(&one));
        // default size is shrunk to a small primary screen
        let small = vec![mon(0, 0, 1280, 680, 1.0, true)];
        assert_eq!(default_placement(&small).size, [1280.0 - FRAME[0], 680.0 - FRAME[1]]);
    }

    #[test]
    fn partly_off_screen_is_clamped_onto_the_screen() {
        let ms = two_screens();
        // hanging off the right edge of the primary (no second screen) -> moved fully onto it
        let one = vec![ms[0]];
        let p = placement(true, Some(&geom(1500, 600, 1200.0, 800.0)), &one);
        assert_eq!((p.pos, p.size), (Some((1920 - 1200 - 16, 1040 - 800 - 40)), [1200.0, 800.0]));
        // bigger than the screen -> shrunk to the work area, placed at its corner
        let p = placement(true, Some(&geom(10, 10, 3000.0, 2000.0)), &one);
        assert_eq!((p.pos, p.size), (Some((0, 0)), [1920.0 - FRAME[0], 1040.0 - FRAME[1]]));
        // tiny / below minimum -> minimum size
        let p = placement(true, Some(&geom(10, 10, 300.0, 200.0)), &one);
        assert_eq!(p.size, MIN_SIZE);
        // negative coordinates are fine when a monitor is there (left of the primary)
        let left = vec![mon(-1920, 0, 1920, 1040, 1.0, false), ms[0]];
        let p = placement(true, Some(&geom(-1800, 50, 1200.0, 800.0)), &left);
        assert_eq!(p.pos, Some((-1800, 50)));
        // scale: logical size on a 150 % screen, clamped to its logical work area
        let p = placement(true, Some(&WindowGeom { scale: 1.5, ..geom(2000, 0, 2000.0, 1200.0) }), &ms);
        assert_eq!(p.size, [2560.0 / 1.5 - FRAME[0], 1400.0 / 1.5 - FRAME[1]]);
        assert_eq!(p.pos, Some((1920, 0)));
    }

    #[test]
    fn option_off_or_reset_uses_defaults() {
        let ms = two_screens();
        let g = geom(100, 80, 1200.0, 800.0);
        assert_eq!(placement(false, Some(&g), &ms), default_placement(&ms), "Remember window layout off");
        assert_eq!(placement(true, None, &ms), default_placement(&ms), "after Reset (file deleted)");
        // reset deletes the file, next start is the default
        let dir = std::env::temp_dir().join(format!("ffenc_lay_{}", crate::util::unique_id()));
        let p = dir.join(ROAMING_FILE);
        Layout { window: Some(g), ..Layout::default() }.save_to(&p).unwrap();
        forget(&p).unwrap();
        assert_eq!(Layout::load_from(&p).window, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_screens_restore_size_only() {
        let p = placement(true, Some(&WindowGeom { maximized: true, ..geom(5000, 5000, 1000.0, 700.0) }), &[]);
        assert_eq!((p.pos, p.size, p.maximized, p.restored), (None, [1000.0, 700.0], true, true));
    }

    #[test]
    fn monitor_lookup() {
        let ms = two_screens();
        assert_eq!(monitor_at(&ms, 2000, 10).unwrap().scale, 1.5);
        assert!(monitor_at(&ms, -50, 10).unwrap().primary, "nearest");
        assert!(monitor_at(&[], 0, 0).is_none());
        let c = centered_on(&ms[0], [1380.0, 900.0]);
        assert_eq!(c, ((1920 - 1380 - 16) / 2, (1040 - 900 - 40) / 2));
    }
}
