//! Codec choice rows (codecs branch): video codec (H.264 default / HEVC / AV1) with codec-specific
//! options, and audio codec (AAC default / AC3 / copy). H.264 + AAC show exactly the 0.5.x rows.

use std::cell::Cell;

use eframe::egui::{self, RichText, Ui};
use ffenc::cmd::codecs::{self, AudioCodec, Encoders, VideoCodec, AC3_BITRATES, CRF_NOTE, FILM_GRAIN_MAX, SVT_PRESET_MAX, SVT_SPEEDS, X265_PRESETS, X265_TUNES};
use ffenc::cmd::common::{AudioEncode, VideoEncode};
use ffenc::ffmpeg::locate::FfPaths;
use ffenc::ffmpeg::probe::MediaInfo;

use super::common::{
    apply_bitrate_menu, audio_filters_ui, bitrate_menu_selection, bitrate_ui, bitrate_ui_ex, hint, named, num_u32,
    show_custom_bitrate_field, video_encode_ui, warn, BitrateMenu,
};
use super::option_tips;

/// Facts from the probed source used for notes and AC3 channel handling.
pub fn source_pix_fmt(info: Option<&MediaInfo>) -> Option<String> {
    info?.of_type("video").next()?.pix_fmt.clone()
}
pub fn source_channels(info: Option<&MediaInfo>) -> Option<u32> {
    info?.of_type("audio").filter_map(|s| s.channels).max()
}

/// In-use FFmpeg (and an optional app-managed copy) for the Using: line and the switch button.
pub struct FfCodecStatus<'a> {
    pub paths: Option<&'a FfPaths>,
    pub encoders: Option<Encoders>,
    pub managed_encoders: Option<Encoders>,
    pub switch_to_managed: Option<&'a Cell<bool>>,
}

impl FfCodecStatus<'_> {
    pub fn none() -> Self {
        Self { paths: None, encoders: None, managed_encoders: None, switch_to_managed: None }
    }

    pub fn offer_switch(&self, encoder: &str) -> bool {
        codecs::can_switch_to_managed(self.paths, self.encoders.as_ref(), self.managed_encoders.as_ref(), encoder)
    }

    pub fn offer_switch_any(&self) -> bool {
        codecs::can_switch_to_managed_for_any(self.paths, self.encoders.as_ref(), self.managed_encoders.as_ref())
    }

    pub fn missing(&self, encoder: &str) -> String {
        codecs::missing_note_for(encoder, self.paths)
    }

    fn disabled_tip(&self, encoder: &str) -> String {
        let note = self.missing(encoder);
        if self.offer_switch(encoder) {
            format!("{note}\n\nThe app-managed FFmpeg has this encoder. Click “Switch to the app-managed FFmpeg” to use it (your own FFmpeg is left as it is).")
        } else {
            note
        }
    }
}

fn using_and_switch_ui(ui: &mut Ui, ff: &FfCodecStatus<'_>) {
    if let Some(p) = ff.paths {
        let (line, hover) = p.using_line(32);
        let r = ui.label(RichText::new(line).small());
        if let Some(h) = hover {
            r.on_hover_text(h);
        }
    }
    if ff.offer_switch_any() && ui.button("Switch to the app-managed FFmpeg").on_hover_text(
        "Point the FFmpeg folder setting at the copy the app downloaded. Your own FFmpeg is not moved or overwritten.",
    ).clicked() {
        if let Some(flag) = ff.switch_to_managed {
            flag.set(true);
        }
    }
}

/// Full H.264 / HEVC / AV1 playback comparison, shown on hover of the selected codec's one-line note.
fn codec_compat_hover() -> String {
    format!(
        "{}\n\n{}\n\n{}",
        VideoCodec::H264.compat_note(),
        VideoCodec::Hevc.compat_note(),
        VideoCodec::Av1.compat_note()
    )
}

/// AV1 Speed: listed useful values plus Custom… (typed box, 0–13). Stored field remains `svt_preset`.
fn av1_speed_ui(ui: &mut Ui, id: &str, preset: &mut u32) {
    let custom_id = ui.id().with(("svt_speed_custom", id));
    let mut custom_open = ui.data(|d| d.get_temp::<bool>(custom_id)).unwrap_or(false);
    let sel = bitrate_menu_selection(*preset, &SVT_SPEEDS, custom_open);
    let selected_text = match sel {
        BitrateMenu::Listed(n) => codecs::svt_speed_label(n),
        BitrateMenu::Custom if SVT_SPEEDS.contains(preset) => "Custom".to_string(),
        BitrateMenu::Custom => format!("Custom {preset}"),
    };
    ui.horizontal(|ui| {
        let combo = egui::ComboBox::from_id_salt(format!("{id}_svtp")).width(260.0).selected_text(&selected_text);
        let r = combo.show_ui(ui, |ui| {
            for &n in &SVT_SPEEDS {
                let on = sel == BitrateMenu::Listed(n);
                if ui.selectable_label(on, codecs::svt_speed_label(n)).clicked() {
                    let (v, c) = apply_bitrate_menu(*preset, Some(n));
                    *preset = v;
                    custom_open = c;
                }
            }
            if ui.selectable_label(sel == BitrateMenu::Custom, "Custom…").clicked() {
                let (v, c) = apply_bitrate_menu(*preset, None);
                *preset = v;
                custom_open = c;
            }
        });
        named(
            r.response,
            egui::WidgetType::ComboBox,
            &format!("Speed: {selected_text}. Choose a listed speed, or Custom… to type a number from 0 to 13."),
        );
        if show_custom_bitrate_field(*preset, &SVT_SPEEDS, custom_open) {
            let nr = num_u32(ui, preset, 0..=SVT_PRESET_MAX, "");
            if nr.changed() && SVT_SPEEDS.contains(preset) {
                custom_open = false;
            }
            named(nr, egui::WidgetType::DragValue, "Speed custom value (0–13)");
        }
    });
    ui.data_mut(|d| d.insert_temp(custom_id, custom_open));
}

/// Video rows inside a 2-column Grid.
pub fn video_codec_ui(ui: &mut Ui, id: &str, v: &mut VideoEncode, src_pix_fmt: Option<&str>, ff: &FfCodecStatus<'_>) {
    video_codec_ui_ex(ui, id, v, src_pix_fmt, None, true, ff);
}

/// Like [`video_codec_ui`], with optional extra hover/hint on the 10-bit checkbox (Frameserver).
pub fn video_codec_ui_ex(ui: &mut Ui, id: &str, v: &mut VideoEncode, src_pix_fmt: Option<&str>, ten_bit_hover: Option<&str>, show_output_fps: bool, ff: &FfCodecStatus<'_>) {
    crate::ui::common::row_label_tip(ui, "Video codec", option_tips::VIDEO_CODEC);
    ui.vertical(|ui| {
        let before = v.codec;
        let enc = ff.encoders.as_ref();
        egui::ComboBox::from_id_salt(format!("{id}_codec")).width(300.0).selected_text(v.codec.label()).show_ui(ui, |ui| {
            for c in VideoCodec::ALL {
                let ok = enc.is_none_or(|e| e.video(c));
                let r = ui.add_enabled(ok, egui::SelectableLabel::new(v.codec == c, c.label()));
                let r = r.on_disabled_hover_text(ff.disabled_tip(c.encoder()));
                if r.clicked() {
                    v.codec = c;
                }
            }
        });
        using_and_switch_ui(ui, ff);
        if v.codec != before {
            // sensible starting values for the new codec (CRF scales differ)
            v.crf = v.codec.default_crf(false);
            v.tune = None;
            if v.codec != VideoCodec::Av1 && !X265_PRESETS.contains(&v.preset.as_str()) {
                v.preset = "slow".into();
            }
        }
        if enc.is_some_and(|e| !e.video(v.codec)) {
            warn(ui, ff.missing(v.codec.encoder()));
        }
        // One-line note for the selected codec; the full comparison is in the hover.
        let note = v.codec.compat_note();
        let r = ui.label(RichText::new(note).small().weak());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, note));
        r.on_hover_text(codec_compat_hover());
    });
    ui.end_row();
    match v.codec {
        VideoCodec::H264 => video_encode_ui(ui, id, v, true, show_output_fps),
        VideoCodec::Hevc | VideoCodec::Av1 => {
            crate::ui::common::row_label_tip(ui, "CRF", option_tips::CRF);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    num_u32(ui, &mut v.crf, v.codec.crf_range(), "");
                    let line = format!(
                        "lower = better quality / bigger file (archive: {} game, {} camera)",
                        v.codec.default_crf(false),
                        v.codec.default_crf(true)
                    );
                    let r = ui.label(RichText::new(&line).small().weak());
                    r.on_hover_text(CRF_NOTE);
                });
            });
            ui.end_row();
            crate::ui::common::row_label_tip(ui, "Speed", option_tips::SPEED);
            if v.codec == VideoCodec::Hevc {
                egui::ComboBox::from_id_salt(format!("{id}_x265p")).selected_text(v.preset.clone()).show_ui(ui, |ui| {
                    for p in X265_PRESETS {
                        ui.selectable_value(&mut v.preset, p.to_string(), p);
                    }
                });
            } else {
                av1_speed_ui(ui, id, &mut v.svt_preset);
            }
            ui.end_row();
            crate::ui::common::row_label_tip(ui, "10-bit", option_tips::TEN_BIT);
            ui.vertical(|ui| {
                let r = ui.checkbox(&mut v.ten_bit, "10-bit output (yuv420p10le)");
                if let Some(h) = ten_bit_hover {
                    r.on_hover_text(h);
                    hint(ui, h);
                }
                hint(ui, "Smoother gradients, less banding, often smaller files. Some older TVs/phones can't play 10-bit.");
                if let Some(n) = codecs::ten_bit_note(src_pix_fmt, v) {
                    hint(ui, n);
                }
            });
            ui.end_row();
            if v.codec == VideoCodec::Hevc {
                crate::ui::common::row_label_tip(ui, "Tune", "Optional x265 tuning for a kind of content (e.g. grain, animation). Leave at none if unsure.");
                let txt = v.tune.clone().unwrap_or_else(|| "none".into());
                egui::ComboBox::from_id_salt(format!("{id}_x265t")).selected_text(txt).show_ui(ui, |ui| {
                    ui.selectable_value(&mut v.tune, None, "none");
                    for t in X265_TUNES {
                        ui.selectable_value(&mut v.tune, Some(t.to_string()), t);
                    }
                });
                ui.end_row();
            } else {
                crate::ui::common::row_label_tip(ui, "Film grain", option_tips::FILM_GRAIN);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        num_u32(ui, &mut v.film_grain, 0..=FILM_GRAIN_MAX, "");
                        let r = ui.add_enabled(v.film_grain > 0, egui::Checkbox::new(&mut v.film_grain_denoise, "Denoise first"));
                        let r = if v.film_grain == 0 {
                            r.on_disabled_hover_text(option_tips::FILM_GRAIN_DENOISE_OFF)
                        } else {
                            r.on_hover_text(option_tips::FILM_GRAIN_DENOISE)
                        };
                        named(r, egui::WidgetType::Checkbox, "Denoise first");
                    });
                    hint(ui, "0 = off. Playback adds synthetic grain; the source is left as-is unless you tick Denoise first.");
                });
                ui.end_row();
            }
        }
    }
    // H.264 shows it in video_encode_ui when requested
    if v.codec != VideoCodec::H264 && show_output_fps {
        super::common::output_fps_ui(ui, &mut v.output_fps);
    }
}

/// Audio rows inside a 2-column Grid. `allow_copy` = false where audio is mixed (join).
pub fn audio_codec_ui(ui: &mut Ui, id: &str, a: &mut AudioEncode, allow_copy: bool, ff: &FfCodecStatus<'_>) {
    audio_codec_ui_ex(ui, id, a, allow_copy, true, ff);
}

/// Like [`audio_codec_ui`]; `show_filters` is false in Settings (gain / compressor have their own default rows).
pub fn audio_codec_ui_ex(ui: &mut Ui, id: &str, a: &mut AudioEncode, allow_copy: bool, show_filters: bool, ff: &FfCodecStatus<'_>) {
    crate::ui::common::row_label_tip(ui, "Audio codec", option_tips::AUDIO_CODEC);
    ui.vertical(|ui| {
        let enc = ff.encoders.as_ref();
        egui::ComboBox::from_id_salt(format!("{id}_acodec")).width(300.0).selected_text(a.codec.label()).show_ui(ui, |ui| {
            for c in AudioCodec::ALL {
                if c == AudioCodec::Copy && !allow_copy {
                    continue;
                }
                let ok = enc.is_none_or(|e| e.audio(c));
                let r = ui.add_enabled(ok, egui::SelectableLabel::new(a.codec == c, c.label())).on_disabled_hover_text(ff.disabled_tip("ac3"));
                if r.clicked() {
                    a.codec = c;
                }
            }
        });
        if ff.offer_switch("ac3") && ui.button("Switch to the app-managed FFmpeg").on_hover_text(
            "Point the FFmpeg folder setting at the copy the app downloaded. Your own FFmpeg is not moved or overwritten.",
        ).clicked() {
            if let Some(flag) = ff.switch_to_managed {
                flag.set(true);
            }
        }
        if !allow_copy && a.codec == AudioCodec::Copy {
            a.codec = AudioCodec::Aac;
        }
        match a.codec {
            AudioCodec::Aac => {}
            AudioCodec::Ac3 => hint(ui, "Dolby Digital: plays on TVs, receivers and Plex; keeps up to 5.1 (7.1 is downmixed to 5.1, stereo stays stereo)."),
            AudioCodec::Copy => hint(ui, "Archive: the original audio tracks are kept bit for bit (no gain/compressor)."),
        }
    });
    ui.end_row();
    match a.codec {
        AudioCodec::Aac => {
            bitrate_ui(ui, "AAC bitrate", &mut a.bitrate_k);
            if show_filters {
                audio_filters_ui(ui, id, a);
            }
        }
        AudioCodec::Ac3 => {
            bitrate_ui_ex(ui, "AC3 bitrate", &mut a.ac3_bitrate_k, &AC3_BITRATES, 96..=640);
            if show_filters {
                audio_filters_ui(ui, id, a);
            }
        }
        AudioCodec::Copy => {}
    }
}

/// Error for a codec the current FFmpeg can't encode (checked when building jobs).
pub fn check_available(enc: Option<&Encoders>, v: Option<&VideoEncode>, a: Option<&AudioEncode>, ff: Option<&FfPaths>) -> Result<(), String> {
    let Some(e) = enc else { return Ok(()) };
    if let Some(v) = v.filter(|v| !e.video(v.codec)) {
        return Err(codecs::missing_note_for(v.codec.encoder(), ff));
    }
    if a.is_some_and(|a| !e.audio(a.codec)) {
        return Err(codecs::missing_note_for("ac3", ff));
    }
    Ok(())
}

#[cfg(test)]
mod speed_menu_tests {
    use super::*;
    use crate::ui::common::{apply_bitrate_menu, bitrate_menu_selection, show_custom_bitrate_field, BitrateMenu};

    #[test]
    fn listed_speeds_show_as_themselves_until_custom_is_chosen() {
        assert_eq!(bitrate_menu_selection(5, &SVT_SPEEDS, false), BitrateMenu::Listed(5));
        assert_eq!(bitrate_menu_selection(2, &SVT_SPEEDS, false), BitrateMenu::Listed(2));
        assert_eq!(bitrate_menu_selection(13, &SVT_SPEEDS, false), BitrateMenu::Listed(13));
        assert!(!show_custom_bitrate_field(5, &SVT_SPEEDS, false));
    }

    #[test]
    fn value_not_in_the_list_is_custom_and_shows_the_box() {
        assert_eq!(bitrate_menu_selection(0, &SVT_SPEEDS, false), BitrateMenu::Custom);
        assert_eq!(bitrate_menu_selection(3, &SVT_SPEEDS, false), BitrateMenu::Custom);
        assert_eq!(bitrate_menu_selection(7, &SVT_SPEEDS, false), BitrateMenu::Custom);
        assert_eq!(bitrate_menu_selection(11, &SVT_SPEEDS, false), BitrateMenu::Custom);
        assert!(show_custom_bitrate_field(3, &SVT_SPEEDS, false));
        assert_eq!(codecs::svt_speed_label(3), "Custom 3");
    }

    #[test]
    fn choosing_custom_keeps_the_stored_speed_and_opens_the_box() {
        assert_eq!(apply_bitrate_menu(5, None), (5, true));
        assert_eq!(bitrate_menu_selection(5, &SVT_SPEEDS, true), BitrateMenu::Custom);
        assert!(show_custom_bitrate_field(5, &SVT_SPEEDS, true));
    }

    #[test]
    fn choosing_a_listed_speed_stores_it_and_closes_custom() {
        assert_eq!(apply_bitrate_menu(3, Some(8)), (8, false));
        assert_eq!(apply_bitrate_menu(5, Some(2)), (2, false));
        assert!(!show_custom_bitrate_field(8, &SVT_SPEEDS, false));
    }
}
