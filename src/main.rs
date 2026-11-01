// No console window on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ui;

use eframe::egui;

/// Window / taskbar icon: the app icon artwork (the same image as the exe's
/// embedded assets/app.ico), at its native 128 px.
const ICON_PNG: &[u8] = include_bytes!("../assets/icon-source.png");

fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(ICON_PNG).unwrap_or_default()
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn window_icon_decodes() {
        let i = super::icon();
        assert_eq!((i.width, i.height), (128, 128));
        assert_eq!(i.rgba.len(), 128 * 128 * 4);
        assert!(i.rgba.chunks(4).any(|p| p[3] == 0), "keeps transparency");
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{} {}", ffenc::APP_NAME, ffenc::APP_VERSION))
            .with_inner_size([1380.0, 900.0])
            .with_min_inner_size([820.0, 600.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(icon())),
        ..Default::default()
    };
    eframe::run_native(ffenc::APP_NAME, options, Box::new(|cc| Ok(Box::new(ui::App::new(cc)))))
}
