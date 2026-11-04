//! Window layout persistence in the UI (0.6.0-dev.2). The pure logic (file format, placement,
//! off-screen checks) is in `ffenc::layout`; this module records the live window/panel/section
//! state, saves it debounced, and applies "Reset window layout".

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use eframe::egui;
use ffenc::layout::{self as fl, DialogRect, Layout, Monitor, Panels, WindowGeom};

/// Changes are written this long after the last change (and on exit).
const DEBOUNCE: Duration = Duration::from_millis(1500);
/// A normal window geometry counts as settled after this long without change.
const STABLE_AFTER: Duration = Duration::from_millis(400);
/// Saved panel sizes are re-applied for at least this long after startup, and until the window
/// size has been unchanged for `RESTORE_SETTLE` (at most `RESTORE_MAX`).
const RESTORE_MIN: Duration = Duration::from_millis(1500);
const RESTORE_SETTLE: Duration = Duration::from_millis(700);
const RESTORE_MAX: Duration = Duration::from_secs(6);

/// Bumped by "Reset window layout": panel and section ids include it, so egui forgets their
/// remembered sizes/states and uses the defaults again.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Open/closed state of remembered sections: seeded from the file at startup, updated as drawn.
static SECTIONS: Mutex<BTreeMap<String, bool>> = Mutex::new(BTreeMap::new());

/// Sizes/positions of resizable dialogs: seeded from the file at startup, updated as shown.
static DIALOGS: Mutex<BTreeMap<String, DialogRect>> = Mutex::new(BTreeMap::new());

fn dialogs() -> std::sync::MutexGuard<'static, BTreeMap<String, DialogRect>> {
    DIALOGS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Give a resizable dialog its remembered size (and position when `movable`) and a stable id.
pub fn dialog_window<'a>(w: egui::Window<'a>, key: &str, movable: bool) -> egui::Window<'a> {
    let mut w = w.id(egui::Id::new(("layout_dialog", key, generation())));
    if let Some(d) = dialogs().get(key).copied() {
        w = w.default_size([d.w, d.h]);
        if movable {
            if let Some([x, y]) = d.pos {
                w = w.default_pos([x, y]);
            }
        }
    }
    w
}

/// Record a dialog's content size (the size egui's `default_size` restores) and its position.
pub fn record_dialog(key: &str, content: egui::Vec2, outer_min: egui::Pos2, movable: bool) {
    let d = DialogRect { w: content.x.round(), h: content.y.round(), pos: movable.then_some([outer_min.x.round(), outer_min.y.round()]) };
    if d.is_sane() {
        let mut map = dialogs();
        if map.get(key) != Some(&d) {
            map.insert(key.to_string(), d);
        }
    }
}

/// Which side a panel is on (to rewrite its remembered rectangle).
#[derive(Clone, Copy)]
pub enum Side {
    Left,
    Right,
    Bottom,
}

fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn sections() -> std::sync::MutexGuard<'static, BTreeMap<String, bool>> {
    SECTIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Id of a resizable panel (changes after a reset, so its width goes back to the default).
pub fn panel_id(name: &str) -> egui::Id {
    egui::Id::new(("layout_panel", name, generation()))
}

/// A collapsible section whose open/closed state is remembered across launches under `key`.
pub fn section<R>(ui: &mut egui::Ui, key: &str, header: egui::CollapsingHeader, default_open: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::CollapsingResponse<R> {
    let open = sections().get(key).copied().unwrap_or(default_open);
    let r = header.id_salt(("layout_section", key, generation())).default_open(open).show(ui, add);
    if let Some(s) = egui::collapsing_header::CollapsingState::load(ui.ctx(), r.header_response.id) {
        let now = s.is_open();
        let mut map = sections();
        if map.get(key) != Some(&now) {
            map.insert(key.to_string(), now);
        }
    }
    r
}

/// Startup decision made in `main` before the window exists.
#[derive(Debug, Clone)]
pub struct Startup {
    pub placement: fl::Placement,
    pub saved: Layout,
}

impl Startup {
    /// Read the settings' option and the layout file and decide where the window opens.
    pub fn decide() -> Startup {
        let remember = ffenc::settings::Settings::load().remember_layout;
        let saved = if remember { Layout::load_from(&fl::layout_path()) } else { Layout::default() };
        let monitors = fl::monitors();
        let placement = fl::placement(remember, saved.window.as_ref(), &monitors);
        Startup { placement, saved }
    }

    pub fn viewport(&self, vb: egui::ViewportBuilder) -> egui::ViewportBuilder {
        let p = &self.placement;
        let mut vb = vb.with_inner_size(p.size);
        if let Some((x, y)) = p.pos {
            vb = vb.with_position([x as f32 / p.scale, y as f32 / p.scale]);
        }
        if p.maximized {
            vb = vb.with_maximized(true);
        }
        vb
    }
}

/// Live layout state owned by the app.
pub struct LayoutRt {
    pub path: PathBuf,
    /// What the window/panels look like now (without the informational monitor field).
    current: Layout,
    /// What was last written (same comparison basis as `current`).
    written: Layout,
    changed_at: Option<Instant>,
    /// Where the window should be at startup (physical px), checked on the first frames
    /// (mixed-DPI setups can land a few pixels off).
    startup_pos: Option<(i32, i32)>,
    frames: u32,
    /// Normal (not maximized) geometry that has been unchanged for a moment. When the window gets
    /// maximized, some systems (Wine, some window managers) report the maximized size a frame or
    /// two before the maximized flag, so the newest normal geometry can't be trusted then.
    stable_normal: Option<WindowGeom>,
    pending_normal: Option<(WindowGeom, Instant)>,
    /// Panel sizes measured this frame.
    pub panels: Panels,
    /// Panel sizes to start from (saved values; cleared on reset).
    pub initial: Panels,
    /// While the window is still settling at startup (size applied late, maximize, DPI change),
    /// egui may clamp a panel to the small first-frame size; until then the saved sizes are
    /// re-applied every frame and nothing is recorded. Ends early when the user presses a button.
    restore_until: Option<Instant>,
    restore_started: Instant,
    last_screen: egui::Vec2,
}

impl LayoutRt {
    pub fn new(startup: Option<Startup>, remember: bool) -> LayoutRt {
        let path = fl::layout_path();
        let (saved, pos) = match startup {
            Some(s) if remember => (s.saved, if s.placement.maximized { None } else { s.placement.pos }),
            _ => (Layout::default(), None),
        };
        *sections() = saved.sections.clone();
        *dialogs() = saved.dialogs.clone();
        let mut cmp = saved.clone();
        if let Some(w) = cmp.window.as_mut() {
            w.monitor = None;
        }
        let stable_normal = cmp.window.clone().map(|w| WindowGeom { maximized: false, ..w });
        LayoutRt { path, current: cmp.clone(), written: cmp, changed_at: None, startup_pos: pos, frames: 0, stable_normal, pending_normal: None, panels: saved.panels.clone(), initial: saved.panels, restore_until: Some(Instant::now() + RESTORE_MIN), restore_started: Instant::now(), last_screen: egui::Vec2::ZERO }
    }

    /// Short description of what was restored (for the log).
    pub fn describe(&self) -> String {
        let p = &self.initial;
        let f = |v: Option<f32>| v.map_or("default".to_string(), |v| format!("{v:.0}"));
        format!(
            "Window layout restored: presets list {}, Source panel {}, queue panel {}, {} section(s), {} dialog size(s).",
            f(p.presets_width),
            f(p.source_width),
            f(p.queue_height),
            sections().len(),
            dialogs().len()
        )
    }

    /// Still re-applying the saved panel sizes (startup)?
    pub fn restoring(&self) -> bool {
        self.restore_until.is_some()
    }

    /// Call at the start of each frame: ends the startup restore phase once the window has settled
    /// or the user starts interacting.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        let Some(until) = self.restore_until else { return };
        let screen = ctx.screen_rect().size();
        let now = Instant::now();
        if (screen - self.last_screen).length() > 0.5 {
            self.last_screen = screen;
            self.restore_until = Some(until.max(now + RESTORE_SETTLE));
        }
        let pressed = ctx.input(|i| i.pointer.any_down());
        if pressed || now >= self.restore_until.unwrap_or(now) || self.restore_started.elapsed() >= RESTORE_MAX {
            self.restore_until = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// During the restore phase, put a panel back to its saved size if egui clamped it to a smaller
    /// first-frame window (only when the saved size fits now).
    pub fn seed_panel(&self, ctx: &egui::Context, id: egui::Id, side: Side, saved: Option<f32>, max: f32) {
        let (Some(want), true) = (saved, self.restoring()) else { return };
        if want > max {
            return;
        }
        if let Some(mut st) = egui::containers::panel::PanelState::load(ctx, id) {
            let r = &mut st.rect;
            let cur = match side {
                Side::Left | Side::Right => r.width(),
                Side::Bottom => r.height(),
            };
            if (cur - want).abs() > 0.5 {
                match side {
                    Side::Left => r.max.x = r.min.x + want,
                    Side::Right => r.min.x = r.max.x - want,
                    Side::Bottom => r.min.y = r.max.y - want,
                }
                ctx.data_mut(|d| d.insert_persisted(id, st));
            }
        }
    }

    /// Record a panel's measured size (not while the saved sizes are still being restored).
    pub fn record(&mut self, which: Side, measured: f32, max: f32) {
        if self.restoring() {
            return;
        }
        let slot = match which {
            Side::Left => &mut self.panels.presets_width,
            Side::Right => &mut self.panels.source_width,
            Side::Bottom => &mut self.panels.queue_height,
        };
        Self::record_panel(slot, measured, max);
    }

    /// Record a panel's measured size. When the panel sits at its upper limit (small window or large text
    /// size), it was squeezed rather than chosen, so a larger saved size is kept for when there's room again.
    pub fn record_panel(slot: &mut Option<f32>, measured: f32, max: f32) {
        let squeezed = measured >= max - 1.0;
        match *slot {
            Some(prev) if squeezed && prev > measured => {}
            _ => *slot = Some(measured),
        }
    }

    /// Current window geometry from egui's viewport info (None while minimized / unknown).
    fn window_now(&mut self, ctx: &egui::Context) -> Option<WindowGeom> {
        let vi = ctx.input(|i| i.viewport().clone());
        if vi.minimized == Some(true) || vi.fullscreen == Some(true) {
            return self.current.window.clone();
        }
        let maximized = vi.maximized.unwrap_or(false);
        if maximized {
            // keep the normal (restored) place/size; just note that it's maximized
            self.pending_normal = None;
            let mut g = self.stable_normal.clone().unwrap_or_else(|| {
                let outer = vi.outer_rect.unwrap_or(egui::Rect::ZERO);
                let ppp = ctx.pixels_per_point();
                WindowGeom { x: (outer.min.x * ppp).round() as i32, y: (outer.min.y * ppp).round() as i32, ..WindowGeom::default() }
            });
            g.maximized = true;
            return Some(g);
        }
        let (outer, inner) = (vi.outer_rect?, vi.inner_rect?);
        let ppp = ctx.pixels_per_point();
        let zoom = ctx.zoom_factor();
        let g = WindowGeom {
            x: (outer.min.x * ppp).round() as i32,
            y: (outer.min.y * ppp).round() as i32,
            // logical px at 100 % text size: points * text zoom
            w: (inner.width() * zoom).round(),
            h: (inner.height() * zoom).round(),
            maximized: false,
            monitor: None,
            scale: vi.native_pixels_per_point.unwrap_or(ppp / zoom),
        };
        if !g.is_sane() {
            return self.current.window.clone();
        }
        match &self.pending_normal {
            Some((p, t)) if *p == g => {
                if t.elapsed() >= STABLE_AFTER {
                    self.stable_normal = Some(g.clone());
                }
            }
            _ => self.pending_normal = Some((g.clone(), Instant::now())),
        }
        if self.stable_normal.as_ref() != Some(&g) {
            ctx.request_repaint_after(STABLE_AFTER);
        }
        Some(g)
    }

    /// Call once per frame after the panels were drawn. Saves (debounced) when something changed.
    pub fn frame(&mut self, ctx: &egui::Context, remember: bool, log: &mut dyn FnMut(String)) {
        self.frames += 1;
        if let Some((x, y)) = self.startup_pos {
            if self.frames <= 6 {
                let ppp = ctx.pixels_per_point();
                if let Some(outer) = ctx.input(|i| i.viewport().outer_rect) {
                    let (ax, ay) = ((outer.min.x * ppp).round() as i32, (outer.min.y * ppp).round() as i32);
                    if (ax - x).abs() > 4 || (ay - y).abs() > 4 {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x as f32 / ppp, y as f32 / ppp)));
                    }
                }
                ctx.request_repaint_after(Duration::from_millis(50));
                return; // don't record the window while it is still being placed
            }
            self.startup_pos = None;
        }
        if !remember {
            return;
        }
        let now = Layout { version: fl::FORMAT_VERSION, window: self.window_now(ctx), panels: self.panels.clone().sanitized(), sections: sections().clone(), dialogs: dialogs().clone() };
        if now != self.current {
            self.current = now;
            self.changed_at = Some(Instant::now());
        }
        if let Some(t) = self.changed_at {
            let left = DEBOUNCE.saturating_sub(t.elapsed());
            if left.is_zero() {
                self.changed_at = None;
                if self.current != self.written {
                    if let Err(e) = self.write() {
                        log(e);
                    }
                }
            } else {
                ctx.request_repaint_after(left);
            }
        }
    }

    fn write(&mut self) -> Result<(), String> {
        let mut out = self.current.clone();
        if let Some(w) = out.window.as_mut() {
            let ms = fl::monitors();
            let (cx, cy) = (w.x + (w.w * w.scale) as i32 / 2, w.y + (w.h * w.scale) as i32 / 2);
            w.monitor = fl::monitor_at(&ms, cx, cy).map(|m| m.work);
        }
        out.save_to(&self.path)?;
        self.written = self.current.clone();
        Ok(())
    }

    /// On exit: write whatever changed since the last save.
    pub fn flush(&mut self, remember: bool) -> Result<(), String> {
        if remember && self.current != self.written {
            self.write()?;
        }
        Ok(())
    }

    /// "Reset window layout": forget the saved file, put panels/sections back to their defaults,
    /// and move the window to the default size, centered on the screen it is on.
    pub fn reset(&mut self, ctx: &egui::Context) -> Result<(), String> {
        GENERATION.fetch_add(1, Ordering::Relaxed);
        sections().clear();
        dialogs().clear();
        self.restore_until = None;
        self.initial = Panels::default();
        self.panels = Panels::default();
        self.current = Layout::default();
        self.written = Layout::default();
        self.changed_at = Some(Instant::now());
        self.startup_pos = None;
        self.stable_normal = None;
        self.pending_normal = None;
        let ppp = ctx.pixels_per_point();
        let zoom = ctx.zoom_factor();
        let ms = fl::monitors();
        let outer = ctx.input(|i| i.viewport().outer_rect).unwrap_or(egui::Rect::ZERO);
        let here = fl::monitor_at(&ms, (outer.center().x * ppp) as i32, (outer.center().y * ppp) as i32).copied();
        let def = fl::default_placement(&here.map(|m| vec![Monitor { primary: true, ..m }]).unwrap_or_default());
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(def.size[0] / zoom, def.size[1] / zoom)));
        if let Some(m) = here {
            let (x, y) = fl::centered_on(&m, def.size);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x as f32 / ppp, y as f32 / ppp)));
        }
        fl::forget(&self.path)
    }

    /// The option was turned off: stop saving and forget what was saved.
    pub fn forget_saved(&mut self) -> Result<(), String> {
        self.written = Layout::default();
        self.changed_at = None;
        fl::forget(&self.path)
    }
}


#[cfg(test)]
mod tests {
    use super::LayoutRt;

    #[test]
    fn squeezed_panels_keep_their_saved_size() {
        let mut s = Some(300.0);
        LayoutRt::record_panel(&mut s, 250.0, 250.0); // at its limit (large text size): keep 300
        assert_eq!(s, Some(300.0));
        LayoutRt::record_panel(&mut s, 200.0, 250.0); // the user made it smaller
        assert_eq!(s, Some(200.0));
        LayoutRt::record_panel(&mut s, 320.0, 500.0); // or larger
        assert_eq!(s, Some(320.0));
        let mut none = None;
        LayoutRt::record_panel(&mut none, 250.0, 250.0);
        assert_eq!(none, Some(250.0));
    }
}
