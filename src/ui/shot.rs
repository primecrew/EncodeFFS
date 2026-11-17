//! Hidden FFENC_SCREENSHOT=<name> helper for release screenshots. Not a user-facing feature.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, ColorImage, RichText};
use ffenc::cmd::codecs::VideoCodec;
use ffenc::cmd::crf;
use ffenc::cmd::remux::{self, Container};
use ffenc::finished::Finished;
use ffenc::settings::Appearance;

use super::common::{self, hint, warn};
use super::fallback::FallbackPrompt;
use super::theme;

pub fn run(kind: &str, out: &Path) -> eframe::Result<()> {
    let kind = kind.to_string();
    let out = out.to_path_buf();
    let viewport = egui::ViewportBuilder::default()
        .with_title("Encode FFS")
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([800.0, 500.0]);
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native(
        "Encode FFS",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx, &Appearance::default());
            Ok(Box::new(ShotApp { kind, out, frames: 0, requested: false, saved: false }))
        }),
    )
}

struct ShotApp {
    kind: String,
    out: PathBuf,
    frames: u32,
    requested: bool,
    saved: bool,
}

fn sample_items() -> Vec<Finished> {
    vec![
        Finished {
            path: PathBuf::from(r"C:\Videos\2025-04-25 17-01-45.mp4"),
            size: Some(42 * 1024 * 1024),
            elapsed: Some(Duration::from_secs(245)),
        },
        Finished {
            path: PathBuf::from(r"C:\Videos\2025-04-25 17-15-02.mp4"),
            size: Some(38 * 1024 * 1024),
            elapsed: Some(Duration::from_secs(198)),
        },
    ]
}


fn shot_name_of(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.rsplit(['\\', '/']).next().unwrap_or(&s).to_string()
}

fn show_finished_old(ctx: &egui::Context, items: &[Finished]) {
    common::fit(
        egui::Window::new("Encoding Finished")
            .id(egui::Id::new("encoding_finished_old"))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
        ctx,
        false,
    )
    .show(ctx, |ui| {
        ui.set_max_width((ctx.screen_rect().width() - 60.0).clamp(240.0, 560.0));
        ui.label(RichText::new(format!("{} files finished", items.len())).strong());
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            egui::Grid::new("finished_list_old").num_columns(2).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                for f in items {
                    let name = shot_name_of(&f.path);
                    ui.vertical(|ui| {
                        ui.label(name);
                        hint(ui, format!("{} · took {}", ffenc::finished::fmt_size(f.size.unwrap()), ffenc::finished::fmt_elapsed(f.elapsed.unwrap())));
                    });
                    let _ = ui.button("Go to file");
                    ui.end_row();
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let _ = ui.add(egui::Button::new(RichText::new("Open folder").strong()));
            let _ = ui.button("Close");
        });
    });
}

fn show_remux_warn(ctx: &egui::Context) {
    common::fit(
        egui::Window::new("Remux")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
        ctx,
        false,
    )
    .show(ctx, |ui| {
        ui.set_min_width(620.0);
        ui.label("Container copy (-map 0 -c copy). Never overwrites; files already in the chosen container are skipped.");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Output container");
            let mut c = Container::Webm;
            for opt in Container::ALL {
                ui.radio_value(&mut c, opt, opt.label());
            }
            let _ = c;
        });
        ui.add_space(4.0);
        let mut on = true;
        ui.checkbox(&mut on, "clip.mkv");
        hint(ui, "clip.mkv");
        let info = {
            use ffenc::ffmpeg::probe::{MediaInfo, StreamInfo};
            MediaInfo {
                streams: vec![
                    StreamInfo { index: 0, codec_type: "video".into(), codec_name: "h264".into(), ..Default::default() },
                    StreamInfo { index: 1, codec_type: "audio".into(), codec_name: "aac".into(), ..Default::default() },
                    StreamInfo { index: 2, codec_type: "subtitle".into(), codec_name: "subrip".into(), ..Default::default() },
                ],
                ..Default::default()
            }
        };
        for w in remux::copy_warnings_for(&info, Container::Webm) {
            warn(ui, format!("clip.mkv: {w}"));
        }
        hint(ui, "All tracks are copied as they are. If a job fails, its source file is kept.");
        ui.add_space(8.0);
        let _ = ui.add(egui::Button::new(RichText::new("Run").strong()));
    });
}

fn show_crf(ctx: &egui::Context) {
    common::fit(
        egui::Window::new("Re-encode video")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
        ctx,
        false,
    )
    .show(ctx, |ui| {
        ui.set_min_width(520.0);
        ui.label(RichText::new("CRF").strong());
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut 22u32).range(0..=51));
            common::crf_help_button(ui, "shot", VideoCodec::H264, Some(1080));
            hint(ui, "lower = better quality / bigger file");
        });
        hint(ui, "1920×1080 source");
    });
    // Force the pop-up open by showing it directly as well, so the screenshot always includes it.
    egui::Window::new("About CRF")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [80.0, 40.0])
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(crf::popup_text(VideoCodec::H264, Some(1080)));
        });
}

impl eframe::App for ShotApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::begin_frame(ctx, &Appearance::default());
        let items = sample_items();
        match self.kind.as_str() {
            "finished-before" => show_finished_old(ctx, &items),
            "finished-after" => {
                // Prefer the real widget; if we cannot inject a popup, fall back to a copy of the new layout.
                show_finished_new_layout(ctx, &items);
            }
            "remux-warn" => show_remux_warn(ctx),
            "fallback-prompt" => {
                // A static ask-stage prompt (no settings needed beyond defaults).
                // Construct via ask() and show; Cancel/Download must not be clicked.
                use std::cell::RefCell;
                thread_local! { static P: RefCell<Option<FallbackPrompt>> = const { RefCell::new(None) }; }
                P.with(|p| {
                    if p.borrow().is_none() {
                        *p.borrow_mut() = Some(FallbackPrompt::ask(
                            "AV1".into(),
                            super::modes::Plan::default(),
                            vec![],
                            None,
                            ffenc::output_guard::WhenExists::Ask,
                        ));
                    }
                    let _ = p.borrow_mut().as_mut().unwrap().show(ctx, &ffenc::settings::Settings::default());
                });
            }
            "crf-popup" => show_crf(ctx),
            other => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label(format!("unknown screenshot kind: {other}"));
                });
            }
        }
        self.frames += 1;
        if self.frames >= 4 && !self.requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        for ev in ctx.input(|i| i.raw.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = ev {
                if save_png(&self.out, &image) {
                    self.saved = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.frames > 120 && !self.saved {
            // fallback: give up
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn show_finished_new_layout(ctx: &egui::Context, items: &[Finished]) {
    let name_of = shot_name_of;
    common::fit(
        egui::Window::new("Encoding Finished")
            .id(egui::Id::new("encoding_finished"))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
        ctx,
        false,
    )
    .show(ctx, |ui| {
        let typ = egui::TextStyle::Body.resolve(ui.style());
        let color = ui.visuals().text_color();
        let measure = |s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_owned(), typ.clone(), color).size().x);
        let btn = measure("Go to file") + ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x + 8.0;
        let mut name_w = measure("2025-04-25 17-01-45.mp4");
        for f in items {
            name_w = name_w.max(measure(&name_of(&f.path)));
        }
        let m = ui.spacing().window_margin;
        let pad = (m.left as f32) + (m.right as f32) + 32.0;
        let max = (ctx.screen_rect().width() - 48.0).max(360.0);
        let w = (name_w + btn + pad).clamp(360.0, max);
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.label(RichText::new(format!("{} files finished", items.len())).strong());
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            ui.set_min_width(w - 16.0);
            egui::Grid::new("finished_list").num_columns(2).spacing([16.0, 8.0]).min_col_width(80.0).striped(true).show(ui, |ui| {
                for f in items {
                    ui.vertical(|ui| {
                        ui.scope(|ui| {
                            ui.set_min_width(name_w);
                            ui.add(egui::Label::new(name_of(&f.path)).wrap_mode(egui::TextWrapMode::Extend));
                        });
                        hint(ui, format!("{} · took {}", ffenc::finished::fmt_size(f.size.unwrap()), ffenc::finished::fmt_elapsed(f.elapsed.unwrap())));
                    });
                    let _ = ui.button("Go to file");
                    ui.end_row();
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let _ = ui.add(egui::Button::new(RichText::new("Open folder").strong()));
            let _ = ui.button("Close");
        });
    });
}

fn save_png(path: &Path, image: &ColorImage) -> bool {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let sz = image.size;
    let raw = image.as_raw();
    match image::RgbaImage::from_raw(sz[0] as u32, sz[1] as u32, raw.to_vec()) {
        Some(img) => img.save(path).is_ok(),
        None => false,
    }
}
