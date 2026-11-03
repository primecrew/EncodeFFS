//! In-app splash overlay: shown for ~1.8 s at startup, gone on any key or click.
//! Static (no animation), readable by screen readers (plain labels), follows theme and text size.

use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use ffenc::splash::{self, SplashText};

/// Built-in picture (placeholder): the app icon.
const DEFAULT_PNG: &[u8] = include_bytes!("../../assets/icon-source.png");

pub struct Splash {
    /// Set on the first frame drawn (startup itself may take a while).
    started: Option<Instant>,
    text: SplashText,
    png: Vec<u8>,
    texture: Option<egui::TextureHandle>,
    pub done: bool,
    /// Where an override was loaded from (for the log).
    pub sources: Vec<String>,
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    if !splash::is_png(bytes) {
        return None;
    }
    let d = eframe::icon_data::from_png_bytes(bytes).ok()?;
    if d.width == 0 || d.height == 0 {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([d.width as usize, d.height as usize], &d.rgba))
}

impl Splash {
    pub fn new() -> Self {
        let dirs = splash::override_dirs();
        let (text, tp) = splash::load_text(&dirs);
        let mut sources = vec![];
        if let Some(p) = tp {
            sources.push(format!("Splash text from {}", p.display()));
        }
        let png = match splash::load_image_override(&dirs) {
            Some((b, p)) if decode(&b).is_some() => {
                sources.push(format!("Splash picture from {}", p.display()));
                b
            }
            Some((_, p)) => {
                sources.push(format!("{} isn't a readable PNG; the built-in picture is used.", p.display()));
                DEFAULT_PNG.to_vec()
            }
            None => DEFAULT_PNG.to_vec(),
        };
        Splash { started: None, text, png, texture: None, done: false, sources }
    }

    /// Draw the overlay. Any key or click dismisses it (the input is consumed).
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.done {
            return;
        }
        let elapsed = self.started.get_or_insert_with(Instant::now).elapsed();
        let dismiss = ctx.input_mut(|i| {
            let key = i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
            let click = i.pointer.any_pressed();
            if key {
                i.events.retain(|e| !matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)));
            }
            key || click
        });
        // test hook: FFENC_SPLASH_MS keeps it up longer (Wine screenshots)
        let ms = std::env::var("FFENC_SPLASH_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(splash::DURATION_MS);
        if dismiss || elapsed >= Duration::from_millis(ms) {
            self.done = true;
            ctx.request_repaint();
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(ms).saturating_sub(elapsed));
        if self.texture.is_none() {
            if let Some(img) = decode(&self.png).or_else(|| decode(DEFAULT_PNG)) {
                self.texture = Some(ctx.load_texture("splash", img, egui::TextureOptions::LINEAR));
            }
        }
        let p = crate::ui::theme::c();
        let screen = ctx.screen_rect();
        // backdrop (also swallows clicks to the app below)
        egui::Area::new(egui::Id::new("splash_backdrop")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let fill = if p.high_contrast { egui::Color32::BLACK } else { ui.visuals().panel_fill.gamma_multiply(0.92) };
            ui.painter().rect_filled(screen, 0.0, fill);
            ui.allocate_rect(screen, egui::Sense::click());
        });
        egui::Area::new(egui::Id::new("splash")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.card_fill)
                .stroke(egui::Stroke::new(if p.high_contrast { 2.0_f32 } else { 1.0 }, if p.high_contrast { p.accent } else { p.card_stroke }))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(24))
                .show(ui, |ui| {
                    ui.set_max_width((screen.width() - 60.0).clamp(200.0, 520.0));
                    ui.vertical_centered(|ui| {
                        if let Some(t) = &self.texture {
                            let s = t.size_vec2();
                            let k = (360.0 / s.x).min(200.0 / s.y).min(1.0);
                            ui.add(egui::Image::new((t.id(), s * k)).alt_text(format!("{} picture", self.text.title)));
                            ui.add_space(8.0);
                        }
                        ui.label(RichText::new(&self.text.title).size(ui.text_style_height(&egui::TextStyle::Heading) * 1.3).strong());
                        ui.label(RichText::new(format!("Version {}{}", ffenc::APP_VERSION, if ffenc::EDITION.is_empty() { String::new() } else { format!(" · {}", ffenc::EDITION) })).color(p.accent));
                        if !self.text.tagline.is_empty() {
                            ui.add_space(4.0);
                            ui.label(&self.text.tagline);
                        }
                        for l in &self.text.body {
                            ui.label(RichText::new(l).small());
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("Press any key or click to continue").small().weak());
                    });
                });
        });
    }
}
