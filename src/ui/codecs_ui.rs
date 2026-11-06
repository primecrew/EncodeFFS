//! Codec choice rows (codecs branch): video codec (H.264 default / HEVC / AV1) with codec-specific
//! options, and audio codec (AAC default / AC3 / copy). H.264 + AAC show exactly the 0.5.x rows.

use eframe::egui::{self, RichText, Ui};
use ffenc::cmd::codecs::{self, AudioCodec, Encoders, VideoCodec, AC3_BITRATES, CRF_NOTE, FILM_GRAIN_MAX, SVT_PRESET_MAX, X265_PRESETS, X265_TUNES};
use ffenc::cmd::common::{AudioEncode, VideoEncode};
use ffenc::ffmpeg::probe::MediaInfo;

use super::common::{audio_encode_ui, audio_filters_ui, hint, named, num_u32, video_encode_ui, warn};

/// Facts from the probed source used for notes and AC3 channel handling.
pub fn source_pix_fmt(info: Option<&MediaInfo>) -> Option<String> {
    info?.of_type("video").next()?.pix_fmt.clone()
}
pub fn source_channels(info: Option<&MediaInfo>) -> Option<u32> {
    info?.of_type("audio").filter_map(|s| s.channels).max()
}

/// "ℹ" button that shows/hides a help text below (keyboard reachable, screen-reader named).
fn info_toggle(ui: &mut Ui, id: &str, name: &str, text: &str) {
    let key = egui::Id::new(("codec_info", id));
    let mut open = ui.data(|d| d.get_temp::<bool>(key).unwrap_or(false));
    let b = named(ui.small_button("ℹ"), egui::WidgetType::Button, name);
    if b.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    if open {
        ui.label(RichText::new(text).small());
    }
}

/// Video rows inside a 2-column Grid. `enc` = encoders of the current FFmpeg (None = unknown yet).
pub fn video_codec_ui(ui: &mut Ui, id: &str, v: &mut VideoEncode, enc: Option<&Encoders>, src_pix_fmt: Option<&str>) {
    crate::ui::common::row_label(ui, "Video codec");
    ui.vertical(|ui| {
        let before = v.codec;
        egui::ComboBox::from_id_salt(format!("{id}_codec")).width(300.0).selected_text(v.codec.label()).show_ui(ui, |ui| {
            for c in VideoCodec::ALL {
                let ok = enc.is_none_or(|e| e.video(c));
                let r = ui.add_enabled(ok, egui::SelectableLabel::new(v.codec == c, c.label()));
                let r = r.on_disabled_hover_text(codecs::missing_note(c.encoder()));
                if r.clicked() {
                    v.codec = c;
                }
            }
        });
        if v.codec != before {
            // sensible starting values for the new codec (CRF scales differ)
            v.crf = v.codec.default_crf(false);
            v.tune = None;
            if v.codec != VideoCodec::Av1 && !X265_PRESETS.contains(&v.preset.as_str()) {
                v.preset = "slow".into();
            }
        }
        if enc.is_some_and(|e| !e.video(v.codec)) {
            warn(ui, codecs::missing_note(v.codec.encoder()));
        }
        ui.horizontal_wrapped(|ui| {
            hint(ui, v.codec.compat_note().split(':').next().unwrap_or(""));
            info_toggle(ui, &format!("{id}_compat"), "Where does this codec play?", &format!("{}\n\n{}\n{}\n{}", v.codec.compat_note(), VideoCodec::H264.compat_note(), VideoCodec::Hevc.compat_note(), VideoCodec::Av1.compat_note()));
        });
    });
    ui.end_row();
    match v.codec {
        VideoCodec::H264 => video_encode_ui(ui, id, v, true),
        VideoCodec::Hevc | VideoCodec::Av1 => {
            crate::ui::common::row_label(ui, "CRF");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    num_u32(ui, &mut v.crf, v.codec.crf_range(), "");
                    hint(ui, format!("lower = better quality / bigger file (archive: {} game, {} camera)", v.codec.default_crf(false), v.codec.default_crf(true)));
                });
                ui.horizontal_wrapped(|ui| {
                    hint(ui, "CRF isn't comparable across codecs.");
                    info_toggle(ui, &format!("{id}_crf"), "About CRF across codecs", CRF_NOTE);
                });
            });
            ui.end_row();
            crate::ui::common::row_label_tip(ui, "Preset", "Encoder speed preset: slower presets compress better (smaller file at the same CRF) but take longer. Does not change the quality target.");
            if v.codec == VideoCodec::Hevc {
                egui::ComboBox::from_id_salt(format!("{id}_x265p")).selected_text(v.preset.clone()).show_ui(ui, |ui| {
                    for p in X265_PRESETS {
                        ui.selectable_value(&mut v.preset, p.to_string(), p);
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    num_u32(ui, &mut v.svt_preset, 0..=SVT_PRESET_MAX, "");
                    hint(ui, "0 = slowest/best … 13 = fastest (5 is a good archive default)");
                });
            }
            ui.end_row();
            crate::ui::common::row_label(ui, "10-bit");
            ui.vertical(|ui| {
                ui.checkbox(&mut v.ten_bit, "10-bit output (yuv420p10le)");
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
                crate::ui::common::row_label(ui, "Film grain");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        num_u32(ui, &mut v.film_grain, 0..=FILM_GRAIN_MAX, "");
                        ui.add_enabled(v.film_grain > 0, egui::Checkbox::new(&mut v.film_grain_denoise, "Denoise first"));
                    });
                    hint(ui, "0 = off. 4–10 re-creates the grain of noisy camera footage instead of spending bits on it.");
                });
                ui.end_row();
            }
        }
    }
}

/// Audio rows inside a 2-column Grid. `allow_copy` = false where audio is mixed (join).
pub fn audio_codec_ui(ui: &mut Ui, id: &str, a: &mut AudioEncode, enc: Option<&Encoders>, allow_copy: bool) {
    crate::ui::common::row_label(ui, "Audio codec");
    ui.vertical(|ui| {
        egui::ComboBox::from_id_salt(format!("{id}_acodec")).width(300.0).selected_text(a.codec.label()).show_ui(ui, |ui| {
            for c in AudioCodec::ALL {
                if c == AudioCodec::Copy && !allow_copy {
                    continue;
                }
                let ok = enc.is_none_or(|e| e.audio(c));
                let r = ui.add_enabled(ok, egui::SelectableLabel::new(a.codec == c, c.label())).on_disabled_hover_text(codecs::missing_note("ac3"));
                if r.clicked() {
                    a.codec = c;
                }
            }
        });
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
        AudioCodec::Aac => audio_encode_ui(ui, id, a),
        AudioCodec::Ac3 => {
            crate::ui::common::row_label(ui, "AC3 bitrate");
            ui.horizontal(|ui| {
                for b in AC3_BITRATES {
                    ui.radio_value(&mut a.ac3_bitrate_k, b, format!("{b} kbps"));
                }
            });
            ui.end_row();
            audio_filters_ui(ui, id, a);
        }
        AudioCodec::Copy => {}
    }
}

/// Error for a codec the current FFmpeg can't encode (checked when building jobs).
pub fn check_available(enc: Option<&Encoders>, v: Option<&VideoEncode>, a: Option<&AudioEncode>) -> Result<(), String> {
    let Some(e) = enc else { return Ok(()) };
    if let Some(v) = v.filter(|v| !e.video(v.codec)) {
        return Err(codecs::missing_note(v.codec.encoder()));
    }
    if a.is_some_and(|a| !e.audio(a.codec)) {
        return Err(codecs::missing_note("ac3"));
    }
    Ok(())
}
