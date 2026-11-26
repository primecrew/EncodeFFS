//! Header strip under the menu bar: the app icon (32 × 32 points) and "Encode FFS v<version>".

use eframe::egui::{self, RichText};

/// The app icon artwork (same picture as assets/app.ico), large enough for 32 points at 300 % text size.
const ICON_PNG: &[u8] = include_bytes!("../../assets/icon-256.png");

/// Icon size in points; the pixel size follows the text size (pixels per point).
pub const ICON_POINTS: f32 = 32.0;

/// "Encode FFS v1.1.0".
pub fn title() -> String {
    format!("{} v{}", ffenc::APP_NAME, ffenc::APP_VERSION)
}

/// The icon resized to `px` × `px` pixels (Lanczos), so it is drawn 1:1 and stays sharp.
pub fn icon_image(px: u32) -> Option<egui::ColorImage> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?;
    let px = px.clamp(8, 512);
    let rgba = img.resize_exact(px, px, image::imageops::FilterType::Lanczos3).to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], rgba.as_raw()))
}

static BOLD_FONT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Font family for the title: Segoe UI Bold, registered by `install_fonts` when Windows has it.
pub fn bold_family() -> egui::FontFamily {
    egui::FontFamily::Name("header-bold".into())
}

pub fn set_bold_font(present: bool) {
    BOLD_FONT.store(present, std::sync::atomic::Ordering::Relaxed);
}

#[derive(Default)]
pub struct Header {
    /// Texture for the current pixel size (rebuilt when the text size changes).
    icon: Option<(u32, egui::TextureHandle)>,
}

impl Header {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let px = (ICON_POINTS * ui.ctx().pixels_per_point()).round().max(8.0) as u32;
        if self.icon.as_ref().map(|(p, _)| *p) != Some(px) {
            self.icon = icon_image(px).map(|img| (px, ui.ctx().load_texture("app-header-icon", img, egui::TextureOptions::LINEAR)));
        }
        let title = title();
        ui.horizontal(|ui| {
            if let Some((_, tex)) = &self.icon {
                ui.add(egui::Image::new((tex.id(), egui::vec2(ICON_POINTS, ICON_POINTS))));
            }
            ui.add_space(4.0);
            let size = ui.style().text_styles.get(&egui::TextStyle::Heading).map_or(20.0, |f| f.size * 1.1);
            let color = ui.visuals().strong_text_color();
            if BOLD_FONT.load(std::sync::atomic::Ordering::Relaxed) {
                ui.label(RichText::new(title).size(size).family(bold_family()).color(color));
            } else {
                // no bold font file: draw the regular face twice, slightly offset (a heavier stroke)
                let galley = ui.painter().layout_no_wrap(title.clone(), egui::FontId::proportional(size), color);
                let off = (size * 0.045).max(0.6);
                let (rect, resp) = ui.allocate_exact_size(galley.size() + egui::vec2(off, 0.0), egui::Sense::hover());
                ui.painter().galley(rect.min, galley.clone(), color);
                ui.painter().galley(rect.min + egui::vec2(off, 0.0), galley, color);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &title));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn title_is_app_name_and_version() {
        assert_eq!(super::title(), format!("Encode FFS v{}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn icon_is_made_at_the_exact_pixel_size_for_each_text_size() {
        for (scale, px) in [(1.0f32, 32usize), (1.25, 40), (2.0, 64), (3.0, 96)] {
            let n = (super::ICON_POINTS * scale).round() as u32;
            let img = super::icon_image(n).expect("icon decodes");
            assert_eq!(img.size, [px, px]);
            assert!(img.pixels.iter().any(|p| p.a() == 0), "keeps transparency");
            assert!(img.pixels.iter().any(|p| p.a() == 255), "has solid pixels");
        }
    }
}
