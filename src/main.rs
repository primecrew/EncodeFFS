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
    // where the window opens: the saved layout (if "Remember window layout" is on and it is still
    // on a connected screen), otherwise the default size, centered
    let startup = ui::layout::Startup::decide();
    let viewport = egui::ViewportBuilder::default()
        .with_title(ffenc::full_title())
        .with_min_inner_size(ffenc::layout::MIN_SIZE)
        .with_drag_and_drop(true)
        .with_icon(std::sync::Arc::new(icon()));
    let options = eframe::NativeOptions { viewport: startup.viewport(viewport), centered: startup.placement.pos.is_none(), ..Default::default() };
    eframe::run_native(ffenc::DATA_DIR_NAME, options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, Some(startup))))))
}
