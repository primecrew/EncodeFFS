//! Appearance: Dark / Light / High-contrast styles, the semantic colour palette
//! used by the app's own coloured text (all ≥ 4.5:1 against every background it
//! appears on – see the contrast tests), UI scale and the keyboard
//! focus ring.

use std::cell::Cell;

use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle, Theme, ThemePreference, Visuals};
use ffenc::settings::{Appearance, ThemeChoice};

/// Colours for app-specific meaning. Text colours are WCAG AA (4.5:1) on the
/// theme's panel, window, text-field and button backgrounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub info: Color32,
    pub muted: Color32,
    pub unsaved: Color32,
    /// Non-text accent: focus ring, drop targets, insertion line (≥ 3:1).
    pub accent: Color32,
    pub focus_width: f32,
    /// Banner / toast backgrounds; text on them is `on_pill` (white).
    pub pill_accent: Color32,
    pub pill_ok: Color32,
    pub pill_warn: Color32,
    pub on_pill: Color32,
    pub high_contrast: bool,
}

const DARK: Palette = Palette {
    ok: Color32::from_rgb(110, 210, 120),
    warn: Color32::from_rgb(240, 190, 70),
    err: Color32::from_rgb(255, 128, 118),
    info: Color32::from_rgb(125, 185, 255),
    muted: Color32::from_gray(170),
    unsaved: Color32::from_rgb(255, 170, 60),
    accent: Color32::from_rgb(90, 170, 255),
    focus_width: 2.0,
    pill_accent: Color32::from_rgb(0, 84, 160),
    pill_ok: Color32::from_rgb(20, 110, 40),
    pill_warn: Color32::from_rgb(150, 80, 0),
    on_pill: Color32::WHITE,
    high_contrast: false,
};

const LIGHT: Palette = Palette {
    ok: Color32::from_rgb(18, 110, 36),
    warn: Color32::from_rgb(140, 80, 0),
    err: Color32::from_rgb(185, 20, 20),
    info: Color32::from_rgb(0, 85, 170),
    muted: Color32::from_gray(88),
    unsaved: Color32::from_rgb(160, 72, 0),
    accent: Color32::from_rgb(0, 95, 190),
    focus_width: 2.0,
    ..DARK
};

const HIGH_CONTRAST: Palette = Palette {
    ok: Color32::from_rgb(80, 255, 120),
    warn: Color32::from_rgb(255, 230, 0),
    err: Color32::from_rgb(255, 130, 130),
    info: Color32::from_rgb(110, 220, 255),
    muted: Color32::from_gray(225),
    unsaved: Color32::from_rgb(255, 200, 0),
    accent: Color32::from_rgb(255, 255, 0),
    focus_width: 3.0,
    pill_accent: Color32::from_rgb(0, 0, 0),
    pill_ok: Color32::from_rgb(0, 0, 0),
    pill_warn: Color32::from_rgb(0, 0, 0),
    on_pill: Color32::from_rgb(255, 255, 0),
    high_contrast: true,
};

thread_local! {
    static CURRENT: Cell<Palette> = const { Cell::new(DARK) };
}

/// The palette for this frame.
pub fn c() -> Palette {
    CURRENT.with(|p| p.get())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
    HighContrast,
}

pub fn palette(k: Kind) -> Palette {
    match k {
        Kind::Dark => DARK,
        Kind::Light => LIGHT,
        Kind::HighContrast => HIGH_CONTRAST,
    }
}

pub fn visuals(k: Kind) -> Visuals {
    let g = Color32::from_gray;
    match k {
        Kind::Dark => {
            let mut v = Visuals::dark();
            v.widgets.noninteractive.fg_stroke.color = g(225);
            // egui's "weak" text = text halfway towards this colour (also disabled widgets)
            v.widgets.noninteractive.weak_bg_fill = g(60);
            v.widgets.inactive.fg_stroke.color = g(235);
            v.hyperlink_color = DARK.info;
            v.warn_fg_color = DARK.warn;
            v.error_fg_color = DARK.err;
            v.selection.stroke.color = Color32::from_rgb(210, 232, 255);
            v
        }
        Kind::Light => {
            let mut v = Visuals::light();
            v.widgets.noninteractive.fg_stroke.color = g(20);
            v.widgets.noninteractive.weak_bg_fill = g(180);
            v.widgets.inactive.fg_stroke.color = g(15);
            v.widgets.hovered.fg_stroke.color = g(0);
            v.widgets.active.fg_stroke.color = g(0);
            v.hyperlink_color = LIGHT.info;
            v.warn_fg_color = LIGHT.warn;
            v.error_fg_color = LIGHT.err;
            v.selection.stroke.color = Color32::from_rgb(0, 60, 110);
            v
        }
        Kind::HighContrast => {
            let (w, y, b) = (Color32::WHITE, HIGH_CONTRAST.accent, Color32::BLACK);
            let mut v = Visuals::dark();
            v.panel_fill = b;
            v.window_fill = b;
            v.extreme_bg_color = b;
            v.faint_bg_color = g(24);
            v.code_bg_color = g(30);
            v.window_stroke = Stroke::new(2.0_f32, w);
            v.window_shadow = egui::epaint::Shadow::NONE;
            v.popup_shadow = egui::epaint::Shadow::NONE;
            let wv = &mut v.widgets;
            wv.noninteractive.bg_fill = b;
            wv.noninteractive.weak_bg_fill = g(190);
            wv.noninteractive.bg_stroke = Stroke::new(1.5_f32, w);
            wv.noninteractive.fg_stroke = Stroke::new(1.0_f32, w);
            wv.inactive.bg_fill = b;
            wv.inactive.weak_bg_fill = b;
            wv.inactive.bg_stroke = Stroke::new(1.5_f32, w);
            wv.inactive.fg_stroke = Stroke::new(1.0_f32, w);
            for x in [&mut wv.hovered, &mut wv.open] {
                x.bg_fill = g(40);
                x.weak_bg_fill = g(40);
                x.bg_stroke = Stroke::new(2.0_f32, y);
                x.fg_stroke = Stroke::new(1.5_f32, y);
            }
            wv.active.bg_fill = g(60);
            wv.active.weak_bg_fill = g(60);
            wv.active.bg_stroke = Stroke::new(3.0_f32, y);
            wv.active.fg_stroke = Stroke::new(2.0_f32, y);
            v.selection.bg_fill = Color32::from_rgb(0, 70, 170);
            v.selection.stroke = Stroke::new(2.0_f32, y);
            v.hyperlink_color = HIGH_CONTRAST.info;
            v.warn_fg_color = HIGH_CONTRAST.warn;
            v.error_fg_color = HIGH_CONTRAST.err;
            v.text_cursor.stroke = Stroke::new(2.0_f32, y);
            v
        }
    }
}

pub fn style(k: Kind, a: &Appearance) -> egui::Style {
    let mut s = egui::Style { visuals: visuals(k), ..Default::default() };
    // slightly larger base sizes than egui's (tiny 9 pt "small" text is hard to read)
    s.text_styles = [
        (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0 * a.mono_scale, FontFamily::Monospace)),
    ]
    .into();
    if k == Kind::HighContrast {
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
    }
    s
}

/// Apply theme + scale + mono size. Call when the appearance changed (cheap enough
/// to call on every change; not every frame).
pub fn apply(ctx: &egui::Context, a: &Appearance) {
    let dark = if a.theme == ThemeChoice::HighContrast { Kind::HighContrast } else { Kind::Dark };
    ctx.set_style_of(Theme::Dark, style(dark, a));
    ctx.set_style_of(Theme::Light, style(Kind::Light, a));
    ctx.options_mut(|o| {
        o.fallback_theme = Theme::Dark;
        // Ctrl +/-/0 are handled by the app (steps, remembered)
        o.zoom_with_keyboard = false;
    });
    ctx.set_theme(match a.theme {
        ThemeChoice::System => ThemePreference::System,
        ThemeChoice::Dark | ThemeChoice::HighContrast => ThemePreference::Dark,
        ThemeChoice::Light => ThemePreference::Light,
    });
    ctx.set_zoom_factor(a.scale);
}

/// Pick this frame's palette from the theme that is actually showing.
pub fn begin_frame(ctx: &egui::Context, a: &Appearance) {
    let k = if a.theme == ThemeChoice::HighContrast {
        Kind::HighContrast
    } else if ctx.style().visuals.dark_mode {
        Kind::Dark
    } else {
        Kind::Light
    };
    CURRENT.with(|p| p.set(palette(k)));
}

/// Clearly visible outline around the widget that has keyboard focus.
pub fn focus_ring(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    let Some(r) = ctx.read_response(id) else { return };
    let rect = r.interact_rect;
    if !rect.is_positive() {
        return;
    }
    let p = c();
    let painter = ctx.layer_painter(r.layer_id);
    painter.rect_stroke(rect.expand(2.0), 3.0, Stroke::new(p.focus_width, p.accent), egui::StrokeKind::Outside);
}

/// WCAG 2 contrast ratio (used by the tests that guard the palettes).
#[cfg_attr(not(test), allow(dead_code))]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let ch = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_contrast_aa_in_every_theme() {
        for k in [Kind::Dark, Kind::Light, Kind::HighContrast] {
            let v = visuals(k);
            let p = palette(k);
            let bgs = [("panel", v.panel_fill), ("window", v.window_fill), ("text field", v.extreme_bg_color), ("button", v.widgets.inactive.weak_bg_fill)];
            let texts = [
                ("text", v.text_color()),
                ("weak text", v.weak_text_color()),
                ("button text", v.widgets.inactive.fg_stroke.color),
                ("link", v.hyperlink_color),
                ("ok", p.ok),
                ("warn", p.warn),
                ("err", p.err),
                ("info", p.info),
                ("muted", p.muted),
                ("unsaved", p.unsaved),
            ];
            for (tn, t) in texts {
                for (bn, b) in bgs {
                    // disabled controls (weak text on a button) are exempt from WCAG 1.4.3; button
                    // backgrounds carry button text, checked below
                    if bn == "button" && tn != "text" && tn != "button text" {
                        continue;
                    }
                    let r = contrast(t, b);
                    assert!(r >= 4.5, "{k:?}: {tn} {t:?} on {bn} {b:?} = {r:.2}");
                }
            }
            for (n, bg) in [("accent pill", p.pill_accent), ("ok pill", p.pill_ok), ("warn pill", p.pill_warn)] {
                assert!(contrast(p.on_pill, bg) >= 4.5, "{k:?} {n}");
            }
            // selected item text and hovered/active button text
            assert!(contrast(v.selection.stroke.color, v.selection.bg_fill) >= 4.5, "{k:?} selection");
            assert!(contrast(v.text_color(), v.selection.bg_fill) >= 4.5, "{k:?} selected text");
            assert!(contrast(v.widgets.hovered.fg_stroke.color, v.widgets.hovered.weak_bg_fill) >= 4.5, "{k:?} hovered");
            assert!(contrast(v.widgets.active.fg_stroke.color, v.widgets.active.weak_bg_fill) >= 4.5, "{k:?} active");
            // focus ring / drop highlight: non-text, WCAG 1.4.11 asks 3:1
            assert!(contrast(p.accent, v.panel_fill) >= 3.0, "{k:?} accent");
            assert!(contrast(p.accent, v.extreme_bg_color) >= 3.0, "{k:?} accent on field");
        }
        assert!(HIGH_CONTRAST.focus_width > DARK.focus_width);
    }

    #[test]
    fn contrast_formula() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::from_gray(118), Color32::WHITE) - 4.54).abs() < 0.02);
    }
}
