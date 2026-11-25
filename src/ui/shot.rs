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
        .with_inner_size(if matches!(kind.as_str(), "crop" | "crop-warn" | "watermark" | "reencode-wm" | "codec-missing" | "codec-gpu" | "trim-section" | "trim-popup") { [1100.0, 900.0] } else { [1100.0, 700.0] })
        .with_min_inner_size([800.0, 500.0]);
    let options = eframe::NativeOptions { viewport, centered: true, ..Default::default() };
    eframe::run_native(
        "Encode FFS",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx, &Appearance::default());
            let live = Live::new(&kind);
            Ok(Box::new(ShotApp { kind, out, frames: 0, requested: false, saved: false, live, started: std::time::Instant::now(), menu_rects: (None, None), menu_scale: std::env::var("FFENC_SHOT_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.1), menu_from: std::env::var("FFENC_SHOT_FROM").ok().and_then(|v| v.parse().ok()) }))
        }),
    )
}

struct ShotApp {
    kind: String,
    out: PathBuf,
    frames: u32,
    requested: bool,
    saved: bool,
    live: Option<Live>,
    started: std::time::Instant,
    /// "choose-size": where the View menu and its Choose size item were drawn (for simulated clicks).
    menu_rects: (Option<egui::Rect>, Option<egui::Rect>),
    menu_scale: f32,
    /// FFENC_SHOT_FROM: open the menu at this size first, close it, switch to menu_scale, reopen.
    menu_from: Option<f32>,
}

/// Real widgets fed with real media (FFENC_SHOT_VIDEO / FFENC_SHOT_IMAGE, FFmpeg from PATH),
/// for the 1.1 crop / watermark screenshots.
struct Live {
    settings: ffenc::settings::Settings,
    ff: Option<ffenc::ffmpeg::locate::FfPaths>,
    mode: Option<Box<dyn super::modes::Mode>>,
    pic: ffenc::cmd::picture::PictureSettings,
    rt: super::picture::PictureRt,
    srcs: Vec<super::picture::PicSource>,
}

impl Live {
    fn new(kind: &str) -> Option<Live> {
        if !matches!(kind, "crop" | "crop-warn" | "watermark" | "reencode-wm" | "codec-missing" | "codec-gpu" | "trim-section" | "trim-popup") {
            return None;
        }
        use ffenc::cmd::picture::{CropSettings, PictureSettings, WatermarkSettings};
        let find = |n: &str| std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join(n)).find(|p| p.is_file());
        let ff = match (find("ffmpeg"), find("ffprobe")) {
            (Some(ffmpeg), Some(ffprobe)) => Some(ffenc::ffmpeg::locate::FfPaths { bin_dir: ffmpeg.parent().unwrap().to_path_buf(), ffmpeg, ffprobe, source: ffenc::ffmpeg::locate::FfSource::SystemPath }),
            _ => None,
        };
        let video = PathBuf::from(std::env::var("FFENC_SHOT_VIDEO").unwrap_or_default());
        let image = std::env::var("FFENC_SHOT_IMAGE").unwrap_or_default();
        let srcs: Vec<super::picture::PicSource> = ff
            .as_ref()
            .and_then(|f| ffenc::ffmpeg::probe::probe(&f.ffprobe, &video).ok())
            .and_then(|i| super::picture::PicSource::from_info(&video, &i))
            .into_iter()
            .collect();
        let crop = match kind {
            "crop" | "watermark" => CropSettings { enabled: true, top: 140, right: 0, bottom: 140, left: 0 },
            "crop-warn" => CropSettings { enabled: true, top: 141, right: 0, bottom: 140, left: 0 },
            _ => CropSettings::default(),
        };
        let watermark = WatermarkSettings { enabled: true, image: image.clone(), x: 0.7375, y: 0.785, w: 0.25, opacity_pct: 70, ..Default::default() };
        let pic = PictureSettings { crop, watermark };
        let mut settings = ffenc::settings::Settings::default();
        if kind == "codec-gpu" {
            // GPU encoding on, with a (fake) working NVIDIA detection for the FFmpeg in use
            settings.hw = fake_hw("settings-hw-on");
            if let Some(f) = &ff {
                settings.hw.detect[0].program = f.ffmpeg.to_string_lossy().into_owned();
            }
        }
        let mode = matches!(kind, "reencode-wm" | "codec-missing" | "codec-gpu" | "trim-section" | "trim-popup").then(|| {
            let mut m = super::modes::new_mode("reencode").unwrap();
            let v = if kind == "codec-gpu" {
                serde_json::json!({
                    "input": video.to_string_lossy(),
                    "out_dir": video.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                    "video": { "codec": "Hevc", "crf": 20 }
                })
            } else if kind == "codec-missing" {
                // AV1 chosen while the FFmpeg in use has no AV1 encoder
                serde_json::json!({
                    "input": video.to_string_lossy(),
                    "out_dir": video.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                    "video": { "codec": "Av1", "crf": 35 }
                })
            } else if kind.starts_with("trim-") {
                // 1:30:00 source, 2:15 cut from the start and 3:00 from the end
                serde_json::json!({
                    "input": video.to_string_lossy(),
                    "out_dir": video.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                    "trim_start": 135.0,
                    "trim_end": 180.0
                })
            } else {
                serde_json::json!({
                    "input": video.to_string_lossy(),
                    "out_dir": video.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                    "picture": { "watermark": { "enabled": true, "image": image, "opacity_pct": 70 } }
                })
            };
            m.load(&v, &settings);
            m
        });
        // "crop" shows the second preview frame so the "2 of 4 (40 %)" label is visible
        let mut rt = super::picture::PictureRt::default();
        rt.frame = usize::from(kind == "crop");
        Some(Live { settings, ff, mode, pic, rt, srcs })
    }

    fn show(&mut self, ctx: &egui::Context, kind: &str) {
        let ffprobe = self.ff.as_ref().map(|f| f.ffprobe.clone());
        let get = std::cell::Cell::new(None);
        let no_av1 = match kind {
            "codec-missing" => Some(ffenc::cmd::codecs::Encoders { libx265: true, libsvtav1: false, ac3: true }),
            "codec-gpu" => Some(ffenc::cmd::codecs::Encoders { libx265: true, libsvtav1: true, ac3: true }),
            _ => None,
        };
        let cx = super::modes::Ctx { settings: &self.settings, ffprobe: ffprobe.as_deref(), egui: ctx, has_xfade: None, encoders: no_av1, ff: self.ff.as_ref(), managed_encoders: None, switch_to_managed: None, get_managed: Some(&get) };
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).vertical_scroll_offset(0.0).show(ui, |ui| match kind {
                "crop" | "crop-warn" => {
                    ui.heading("Re-encode video");
                    super::picture::crop_section(ui, &cx, "shot", &mut self.pic.crop, &self.srcs, &mut self.rt);
                }
                "watermark" => {
                    ui.heading("Add Watermark");
                    common::section(ui, "2. Watermark");
                    super::picture::watermark_editor(ui, &cx, "shot", &mut self.pic, &self.srcs, &mut self.rt);
                }
                "reencode-wm" | "codec-missing" | "codec-gpu" | "trim-section" | "trim-popup" => {
                    if let Some(m) = self.mode.as_mut() {
                        ui.heading(m.title());
                        m.ui(ui, &cx);
                    }
                }
                _ => {}
            });
        });
        if kind == "trim-popup" {
            // the pop-up the From Start box raises when "1:75" is typed and Enter is pressed
            let e = ffenc::timeparse::parse_time("1:75").unwrap_err();
            common::time_popup(ctx, format!("{e}\n\nThe box goes back to 2:15."));
        }
        common::time_popup_ui(ctx);
        ctx.request_repaint();
    }
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

fn show_remux_after_new(ctx: &egui::Context) {
    let def = std::env::temp_dir().join("ffenc-shot-default");
    let rec = std::env::temp_dir().join("ffenc-shot-recent");
    let _ = std::fs::create_dir_all(&def);
    let _ = std::fs::create_dir_all(&rec);
    let def_s = def.to_string_lossy().into_owned();
    let recents = vec![rec.to_string_lossy().into_owned()];
    let mut src = String::new();
    let mut out = String::new();
    common::fit(
        egui::Window::new("Remux")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
        ctx,
        false,
    )
    .show(ctx, |ui| {
        ui.set_min_width(720.0);
        ui.label("Container copy (-map 0 -c copy). Never overwrites; files already in the chosen container are skipped.");
        ui.add_space(6.0);
        egui::Grid::new("shot_remux_new").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let _ = common::folder_row(ui, "shot_remux_src", "Source folder", &mut src, &def_s, &recents, true, "");
            let _ = common::folder_row_nodrop(ui, "shot_remux_out", "Output folder", &mut out, &def_s, &recents, "(same as source folder)");
            crate::ui::common::row_label(ui, "Output container");
            ui.horizontal(|ui| {
                let mut c = Container::Mp4;
                for opt in Container::ALL {
                    ui.radio_value(&mut c, opt, opt.label());
                }
                let _ = c;
            });
            ui.end_row();
        });
        hint(ui, "All tracks are copied as they are. If a job fails, its source file is kept.");
        ui.add_space(8.0);
        let _ = ui.add(egui::Button::new(RichText::new("Preview & run…").strong()));
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


/// A fake detection for the 1.1 “Encoding hardware” screenshots (no real GPU needed).
fn fake_hw(kind: &str) -> ffenc::cmd::hwenc::HwSettings {
    use ffenc::cmd::hwenc::{HwDetect, HwSettings, HwTest, HwVendor};
    let failed = kind == "settings-hw-failed";
    let tests = VideoCodec::ALL
        .iter()
        .map(|&codec| HwTest { codec, vendor: HwVendor::Nvidia, ok: !failed, ok_10bit: !failed, error: if failed { "Cannot load nvcuda.dll".into() } else { String::new() } })
        .collect();
    let det = HwDetect { program: "ffmpeg.exe".into(), tests, gpu_name: (!failed).then(|| "NVIDIA GeForce RTX 4070".into()), checked: 1 };
    HwSettings { use_gpu: kind == "settings-hw-on", detect: if kind == "settings-hw-new" { vec![] } else { vec![det] }, nudge_done: false }
}

fn show_settings_hw(ctx: &egui::Context, kind: &str) {
    let hw = fake_hw(kind);
    thread_local! { static USE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
    let mut use_gpu = USE.with(|u| u.get()).unwrap_or(hw.use_gpu);
    common::fit(egui::Window::new("Settings").collapsible(false).resizable(false).default_width(640.0).anchor(egui::Align2::CENTER_TOP, [0.0, 20.0]), ctx, false).show(ctx, |ui| {
        ui.set_min_width(620.0);
        common::section(ui, "Encoding hardware");
        let _ = super::hw::section_ui(ui, &mut use_gpu, &hw, Some(Path::new("ffmpeg.exe")), None, kind == "settings-hw-checking");
        common::section(ui, "Encoding defaults (used by built-in presets that are not customised, and by new blank presets)");
        hint(ui, "…");
    });
    USE.with(|u| u.set(Some(use_gpu)));
    if kind == "settings-hw-nudge" {
        egui::Window::new("Encoding hardware").collapsible(false).resizable(false).default_width(520.0).anchor(egui::Align2::CENTER_CENTER, [0.0, 120.0]).show(ctx, |ui| {
            let _ = super::hw::nudge_ui(ui, &hw.detect[0]);
        });
    }
}

fn show_about(ctx: &egui::Context) {
    common::fit(
        egui::Window::new("About")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]),
        ctx,
        false,
    )
    .show(ctx, |ui| {
        ui.set_min_width(520.0);
        ui.heading(ffenc::about_heading());
        ui.label(ffenc::ABOUT_TAGLINE);
        ui.label(format!("Build: {} {}", std::env::consts::OS, std::env::consts::ARCH));
        ui.label("Licensed under GPL v3");
    });
}

impl eframe::App for ShotApp {
    /// "choose-size": open View, then hover Choose size, like a user would.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.kind != "choose-size" {
            return;
        }
        let (view, size) = self.menu_rects;
        let mut f = self.frames;
        if self.menu_from.is_some() {
            if f == 12 {
                raw.events.push(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
                return;
            }
            if f >= 13 {
                f -= 13; // second round at the new size
            }
        }
        match (f, view, size) {
            (3, Some(v), _) => {
                let p = v.center();
                raw.events.push(egui::Event::PointerMoved(p));
                raw.events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
            }
            (4, Some(v), _) => {
                raw.events.push(egui::Event::PointerButton { pos: v.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
            }
            (f, _, Some(r)) if f >= 6 => {
                // move in from the left edge of the item, then rest on it
                let p = if f == 6 { r.left_center() + egui::vec2(8.0, 0.0) } else { r.center() };
                raw.events.push(egui::Event::PointerMoved(p));
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.kind == "choose-size" {
            // default 110 % (FFENC_SHOT_SCALE to change); the submenu is the real View › Choose size one
            let first = self.menu_from.filter(|_| self.frames < 13).unwrap_or(self.menu_scale);
            let a = Appearance { scale: first, ..Appearance::default() };
            if self.frames == 0 || self.frames == 13 {
                theme::apply(ctx, &a);
                self.menu_rects.1 = None;
            }
            theme::begin_frame(ctx, &a);
            egui::TopBottomPanel::top("shot_menu").show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    let mut size = None;
                    let v = ui.menu_button("View", |ui| {
                        common::one_line_menu(ui);
                        ui.label(RichText::new(format!("Text size: {:.0} %", first * 100.0)).strong());
                        let _ = ui.button("Larger      Ctrl +");
                        let _ = ui.button("Smaller     Ctrl −");
                        let _ = ui.button("100 %        Ctrl+0");
                        let mut sc = first;
                        size = Some(common::choose_size_menu(ui, &mut sc).rect);
                    });
                    self.menu_rects.0 = Some(v.response.rect);
                    if size.is_some() {
                        self.menu_rects.1 = size;
                    }
                });
            });
            egui::CentralPanel::default().show(ctx, |_| {});
            self.frames += 1;
            let shoot_at = if self.menu_from.is_some() { 30 } else { 14 };
            if self.frames >= shoot_at && !self.requested {
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
            ctx.request_repaint();
            if self.frames > 200 && !self.saved {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        theme::begin_frame(ctx, &Appearance::default());
        let items = sample_items();
        match self.kind.as_str() {
            "finished-before" => show_finished_old(ctx, &items),
            "finished-after" => {
                // Prefer the real widget; if we cannot inject a popup, fall back to a copy of the new layout.
                show_finished_new_layout(ctx, &items);
            }
            "remux-warn" => show_remux_warn(ctx),
            "remux-new" => show_remux_after_new(ctx),
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
                        ));
                    }
                    let _ = p.borrow_mut().as_mut().unwrap().show(ctx, &ffenc::settings::Settings::default());
                });
            }
            "crf-popup" => show_crf(ctx),
            "about" => show_about(ctx),
            k if k.starts_with("settings-hw") => {
                let k = k.to_string();
                show_settings_hw(ctx, &k);
            }
            k if self.live.is_some() => {
                let k = k.to_string();
                self.live.as_mut().unwrap().show(ctx, &k);
            }
            other => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label(format!("unknown screenshot kind: {other}"));
                });
            }
        }
        self.frames += 1;
        let wait = if self.live.is_some() { std::time::Duration::from_secs(5) } else { std::time::Duration::ZERO };
        if self.frames >= 4 && self.started.elapsed() >= wait && !self.requested {
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
        if self.frames > 120 && self.started.elapsed() > wait + std::time::Duration::from_secs(10) && !self.saved {
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
