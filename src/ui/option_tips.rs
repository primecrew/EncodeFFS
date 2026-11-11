//! Plain-words hover help for technical option-row labels.
//!
//! Used with [`crate::ui::common::row_label_tip`]. Kept in one place so the same
//! wording can be reused if this is later brought into other editions.

/// Quality (CRF) on H.264 and on HEVC/AV1.
pub const CRF: &str = "Quality of the compressed picture: a lower number means better quality and a larger file. The usual default is 22 for H.264; other codecs use a different scale, so their numbers are not comparable.";

pub const VIDEO_CODEC: &str = "How the picture is compressed. H.264 is the default and plays almost everywhere; HEVC (H.265) and AV1 can make smaller files but need a player that supports them.";

pub const AUDIO_CODEC: &str = "How the sound is compressed: AAC is the default and plays everywhere; AC3 (Dolby Digital) suits TVs and home cinema and keeps up to 5.1 channels. Keep original copies the source audio without changing it.";

pub const TEN_BIT: &str = "Records colour with 10 bits per channel instead of 8, which can smooth gradients and reduce banding. It is on by default for HEVC and AV1 (H.264 stays 8-bit); turn it off if you need to play the file on older TVs or phones.";

pub const SPEED: &str = "Slower takes longer and makes a smaller file at the same CRF. Try a short clip at two speeds and compare.";

pub const FILM_GRAIN: &str = "How much synthetic grain AV1 should add when the file is played. 0 is off; try 4–10 for grainy camera material.";

/// Hover on the “Denoise first” checkbox (disabled when film grain is 0).
pub const FILM_GRAIN_DENOISE: &str = "Cleans the source before encoding so the file stays smaller; grain at playback is fully synthetic. Leave this off to keep the real noise and still signal grain on top.";

/// Disabled “Denoise first” when film grain is off.
pub const FILM_GRAIN_DENOISE_OFF: &str = "Turn film grain on first. Denoise only applies when grain is being signaled.";

pub const COLOUR_RANGE: &str = "Converts full-range (PC) video to TV (limited) range and tags the file that way. Most game captures and camera files are already TV range. Squeezing twice gives grey blacks — check a dark scene, and leave this off unless the source is full range.";

pub const FASTSTART: &str = "Moves the MP4 index to the front so playback can start before the whole file loads (web, Plex over network, phones). Adds a short rewrite at the end. MP4/MOV only.";

pub const CONTAINER: &str = "The kind of output file: MP4 plays in more places; MKV can keep more kinds of audio, subtitles and attachments. Where Auto is offered, MKV inputs stay MKV and everything else becomes MP4.";

pub const DEINTERLACE: &str = "Turns interlaced video (the old “combed” look) into normal progressive frames. It only runs when a file is detected as interlaced; leave it on unless you have a reason not to.";

pub const RESIZE: &str = "Changes the height of the picture; the width stays in proportion. Leave this off to keep the original size.";

/// Start/end trim in seconds (re-encode, batch, custom).
pub const TRIM: &str = "How many seconds to cut from the start and from the end of the file. Leave both at 0 to keep the whole clip.";

/// Frameserver trim is in frame numbers, not seconds.
pub const TRIM_FRAMES: &str = "Encode only part of the timeline, by first and last frame number. Leave this off to use the whole clip.";

pub const COLOUR_FORMAT: &str = "Converts the picture to a colour layout in the AviSynth script. YUV 4:2:0 is the usual choice and plays everywhere. Change this only if a filter needs a different layout.";

pub const COLOUR_MATRIX: &str = "Vegas 8-bit projects keep video levels as-is, so PC.709 avoids a second squeeze. Use Rec709 for 32-bit full-range Vegas projects. Grey blacks usually mean the picture was squeezed twice.";

pub const SIGNPOST_FORMAT: &str = "The colour layout requested from the frameserver. Leave this on Automatic unless the picture looks skewed or the colours look wrong.";

pub const FRAMESERVE_AUDIO: &str = "Whether the script keeps the audio from the signpost, converts it to 16-bit if the audio fails, or drops audio for a video-only encode. Keep the audio is the usual choice.";

/// Chapter mode in the chapter creator (re-encode, frameserver, and similar forms).
pub const CHAPTERS: &str = "Whether to add chapter markers: none, automatic at a set interval, a list you type, or both. The default is no chapters.";

pub const INTERVAL: &str = "How often automatic chapters are added, in minutes. The usual default is 5 minutes.";

pub const CHAPTERS_BATCH: &str = "Optionally add chapter markers to files that don't have any, at a set interval. Files that already have chapters keep theirs. Leave this off unless you want markers added.";

pub const CHAPTERS_JOIN: &str = "Adds a chapter marker at the start of each source file, named after that file. This is on by default; turn it off if you don't want chapter markers.";

/// Every required option-row label and the tip that must appear on it.
/// “Audio” is Frameserver only; “Chapters” and “Interval” include the chapter creator.
#[cfg(test)]
pub const LISTED_ROWS: &[(&str, &str)] = &[
    ("CRF", CRF),
    ("Video codec", VIDEO_CODEC),
    ("Audio codec", AUDIO_CODEC),
    ("10-bit", TEN_BIT),
    ("Speed", SPEED),
    ("Film grain", FILM_GRAIN),
    ("Colour range", COLOUR_RANGE),
    ("Faststart", FASTSTART),
    ("Container", CONTAINER),
    ("Deinterlace", DEINTERLACE),
    ("Resize", RESIZE),
    ("Trim", TRIM),
    ("Colour format", COLOUR_FORMAT),
    ("Colour matrix", COLOUR_MATRIX),
    ("Signpost format", SIGNPOST_FORMAT),
    ("Audio", FRAMESERVE_AUDIO),
    ("Chapters", CHAPTERS),
    ("Interval", INTERVAL),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn ui_rust_files() -> Vec<(String, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let mut out = vec![];
        fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
            for e in fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                    let rel = p.strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap().display().to_string();
                    out.push((rel, fs::read_to_string(&p).unwrap()));
                }
            }
        }
        walk(&root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Mask `row_label_tip(...)` so a search for `row_label(` does not match it.
    fn without_tips(src: &str) -> String {
        let mut s = src.to_string();
        while let Some(i) = s.find("row_label_tip") {
            let rest = &s[i + "row_label_tip".len()..];
            let end = rest.find(')').map(|j| i + "row_label_tip".len() + j + 1).unwrap_or(s.len());
            s.replace_range(i..end, "/*tip*/");
        }
        s
    }

    fn plain_label(src: &str, label: &str) -> bool {
        let pat = format!("row_label(ui, \"{label}\")");
        without_tips(src).contains(&pat)
    }

    fn tipped_label(src: &str, label: &str) -> bool {
        src.contains(&format!("row_label_tip(ui, \"{label}\""))
    }

    #[test]
    fn each_listed_row_has_a_tip() {
        fn sentences(tip: &str) -> usize {
            // Count sentence endings, not abbreviation dots in H.264 or 5.1.
            tip.split(". ").filter(|p| !p.trim().is_empty()).count()
        }
        for (name, tip) in LISTED_ROWS {
            assert!(tip.len() >= 40, "{name}: tip is too short");
            let n = sentences(tip);
            assert!((1..=3).contains(&n), "{name}: expected one or two sentences, got {n}: {tip}");
        }
        // Variant tips used on the same label in other modes must also be real sentences.
        for (name, tip) in [
            ("Trim (frames)", TRIM_FRAMES),
            ("Chapters (batch)", CHAPTERS_BATCH),
            ("Chapters (join)", CHAPTERS_JOIN),
            ("Denoise first", FILM_GRAIN_DENOISE),
            ("Denoise first (off)", FILM_GRAIN_DENOISE_OFF),
        ] {
            assert!(tip.len() >= 40, "{name}: tip is too short");
            let n = sentences(tip);
            assert!((1..=3).contains(&n), "{name}: expected one or two sentences, got {n}: {tip}");
        }

        let files = ui_rust_files();
        // These labels should never appear as a plain row_label in the UI.
        let always = [
            "CRF",
            "Video codec",
            "Audio codec",
            "10-bit",
            "Speed",
            "Film grain",
            "Colour range",
            "Faststart",
            "Container",
            "Deinterlace",
            "Resize",
            "Trim",
            "Colour format",
            "Colour matrix",
            "Signpost format",
            "Interval",
        ];
        for (path, src) in &files {
            for label in always {
                assert!(!plain_label(src, label), "{path}: {label} still uses row_label without a tip");
                if src.contains(&format!("\"{label}\"")) && (src.contains("row_label(") || src.contains("row_label_tip(")) {
                    // If this file has the option row, it must use the tip helper.
                    if tipped_label(src, label) || without_tips(src).matches(&format!("\"{label}\"")).count() == 0 {
                        // ok
                    }
                    if src.contains(&format!("row_label(ui, \"{label}\"")) && !src.contains("row_label_tip") {
                        panic!("{path}: {label} is a plain row_label");
                    }
                }
            }
        }

        // Frameserver Audio must be tipped; Custom pipeline's "Audio" action row stays untipped.
        let fs = files.iter().find(|(p, _)| p.ends_with("modes/frameserve.rs")).expect("frameserve.rs");
        assert!(tipped_label(&fs.1, "Audio"), "Frameserver Audio row must use row_label_tip");
        assert!(!plain_label(&fs.1, "Audio"));
        assert!(fs.1.contains("option_tips::FRAMESERVE_AUDIO"), "Frameserver Audio should use the shared tip constant");

        let custom = files.iter().find(|(p, _)| p.ends_with("modes/custom.rs")).expect("custom.rs");
        assert!(plain_label(&custom.1, "Audio"), "Custom pipeline Audio action is not the Frameserver Audio setting and should not get that tip");

        let creator = files.iter().find(|(p, _)| p.ends_with("chapter_creator.rs")).expect("chapter_creator.rs");
        assert!(tipped_label(&creator.1, "Chapters"), "chapter_creator Chapters must have a tip");
        assert!(tipped_label(&creator.1, "Interval"), "chapter_creator Interval must have a tip");
        assert!(!plain_label(&creator.1, "Chapters"));
        assert!(!plain_label(&creator.1, "Interval"));
    }

    #[test]
    fn shared_constants_are_used_on_the_listed_rows() {
        let files = ui_rust_files();
        let joined = files.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join("\n");
        for name in [
            "option_tips::CRF",
            "option_tips::VIDEO_CODEC",
            "option_tips::AUDIO_CODEC",
            "option_tips::TEN_BIT",
            "option_tips::SPEED",
            "option_tips::FILM_GRAIN",
            "option_tips::FILM_GRAIN_DENOISE",
            "option_tips::FILM_GRAIN_DENOISE_OFF",
            "option_tips::COLOUR_RANGE",
            "option_tips::FASTSTART",
            "option_tips::CONTAINER",
            "option_tips::DEINTERLACE",
            "option_tips::RESIZE",
            "option_tips::TRIM",
            "option_tips::TRIM_FRAMES",
            "option_tips::COLOUR_FORMAT",
            "option_tips::COLOUR_MATRIX",
            "option_tips::SIGNPOST_FORMAT",
            "option_tips::FRAMESERVE_AUDIO",
            "option_tips::CHAPTERS",
            "option_tips::INTERVAL",
            "option_tips::CHAPTERS_BATCH",
            "option_tips::CHAPTERS_JOIN",
        ] {
            assert!(joined.contains(name), "expected a call site to use {name}");
        }
        // Both CRF rows (H.264 form and HEVC/AV1 form) must use the shared string.
        let crf_uses = joined.matches("option_tips::CRF").count();
        assert!(crf_uses >= 2, "CRF tip must be used in common.rs and codecs_ui.rs, found {crf_uses}");
    }
}
