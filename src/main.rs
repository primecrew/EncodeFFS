// No console window on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ui;

use eframe::egui;

/// Simple generated window icon (rounded square with a play triangle).
fn icon() -> egui::IconData {
    let n = 64u32;
    let mut rgba = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let i = ((y * n + x) * 4) as usize;
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // rounded square
            let r = 12.0;
            let cx = fx.clamp(r, n as f32 - r);
            let cy = fy.clamp(r, n as f32 - r);
            let inside = (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r;
            if !inside {
                continue;
            }
            // play triangle
            let tri = fx >= 22.0 && fx <= 46.0 && (fy - 32.0).abs() <= (46.0 - fx) * 0.6;
            let c: [u8; 4] = if tri { [255, 255, 255, 255] } else { [30, 120 + (y as u8 / 2), 60, 255] };
            rgba[i..i + 4].copy_from_slice(&c);
        }
    }
    egui::IconData { rgba, width: n, height: n }
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
