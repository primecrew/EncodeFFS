//! Resolution-aware CRF suggestions. Suggest only — never change the user's value.
//!
//! Ranges come from HandBrake's official "Adjusting quality" documentation
//! (https://handbrake.fr/docs/en/latest/workflow/adjust-quality.html):
//!   * x264 and x265 share one table (RF 18–22 SD, 19–23 720p, 20–24 1080p, 22–28 4K).
//!   * SVT-AV1 has its own table (RF 22–32 SD, 25–35 720p and 1080p, 25–40 4K).
//! SVT-AV1's own FFmpeg guide says a good starting point for 1080p is CRF 30, which
//! sits in HandBrake's 1080p range. x265's official default CRF is 28 (its docs do
//! not publish resolution-specific ranges); the suggested ranges here follow HandBrake.

use super::codecs::VideoCodec;

/// Published starting ranges, keyed by a coarse source-size bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeClass {
    /// 480p/576p and smaller (height ≤ 576).
    Sd,
    /// 720p (height 577–720).
    Hd720,
    /// 1080p class (height 721–2159). Heights between 1080 and 4K (e.g. 1440p)
    /// use HandBrake's 1080p figures — the closest published guide; there is no
    /// separate official 1440p range.
    Hd1080,
    /// 2160p / 4K and taller (height ≥ 2160).
    Uhd4k,
}

impl SizeClass {
    pub fn from_height(height: u32) -> Self {
        if height <= 576 {
            SizeClass::Sd
        } else if height <= 720 {
            SizeClass::Hd720
        } else if height < 2160 {
            SizeClass::Hd1080
        } else {
            SizeClass::Uhd4k
        }
    }

    /// Short label for the pop-up, e.g. "1080p".
    pub fn label(self) -> &'static str {
        match self {
            SizeClass::Sd => "SD (480p/576p)",
            SizeClass::Hd720 => "720p",
            SizeClass::Hd1080 => "1080p",
            SizeClass::Uhd4k => "4K (2160p)",
        }
    }

    /// True when this height is not one of the four sizes HandBrake names
    /// (so the pop-up should say we are using the closest published guide).
    pub fn is_named_height(self, height: u32) -> bool {
        match self {
            SizeClass::Sd => height <= 576,
            SizeClass::Hd720 => height == 720,
            SizeClass::Hd1080 => height == 1080,
            SizeClass::Uhd4k => height >= 2160,
        }
    }
}

/// Inclusive suggested CRF range for a codec at a source size, if published.
pub fn suggested_range(codec: VideoCodec, class: SizeClass) -> (u32, u32) {
    match codec {
        VideoCodec::H264 | VideoCodec::Hevc => match class {
            SizeClass::Sd => (18, 22),
            SizeClass::Hd720 => (19, 23),
            SizeClass::Hd1080 => (20, 24),
            SizeClass::Uhd4k => (22, 28),
        },
        VideoCodec::Av1 => match class {
            SizeClass::Sd => (22, 32),
            SizeClass::Hd720 | SizeClass::Hd1080 => (25, 35),
            SizeClass::Uhd4k => (25, 40),
        },
    }
}

fn codec_words(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "H.264",
        VideoCodec::Hevc => "HEVC",
        VideoCodec::Av1 => "AV1",
    }
}

/// Plain-language pop-up body. Never tells the app to change the user's CRF.
pub fn popup_text(codec: VideoCodec, height: Option<u32>) -> String {
    let mut s = String::from(
        "CRF is the quality setting. A lower number means better quality and a bigger file. A higher number means more compression and a smaller file.\n\nCRF numbers are not comparable across H.264, HEVC and AV1.",
    );
    match height {
        Some(h) if h > 0 => {
            let class = SizeClass::from_height(h);
            let (lo, hi) = suggested_range(codec, class);
            s.push_str("\n\n");
            if class == SizeClass::Hd1080 && h != 1080 {
                s.push_str(&format!(
                    "This video is {h} pixels tall. There is no published range for that exact size; the closest official guide is HandBrake's {} range for {}: {lo}–{hi}.",
                    class.label(),
                    codec_words(codec),
                ));
            } else {
                s.push_str(&format!(
                    "For this {} video, a usual starting range for {} is {lo}–{hi}.",
                    class.label(),
                    codec_words(codec),
                ));
            }
        }
        _ => {
            s.push_str("\n\nAdd a video to see a suggested range for its size. These are starting points only.");
        }
    }
    s.push_str("\n\nThis is only a suggestion. Your CRF value is not changed.");
    s
}

/// Height of the first real video stream (ignores attached pictures).
pub fn source_height(width_height: Option<(u32, u32)>) -> Option<u32> {
    width_height.and_then(|(_, h)| (h > 0).then_some(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_follow_handbrake_sizes() {
        assert_eq!(SizeClass::from_height(480), SizeClass::Sd);
        assert_eq!(SizeClass::from_height(576), SizeClass::Sd);
        assert_eq!(SizeClass::from_height(720), SizeClass::Hd720);
        assert_eq!(SizeClass::from_height(1080), SizeClass::Hd1080);
        assert_eq!(SizeClass::from_height(1440), SizeClass::Hd1080);
        assert_eq!(SizeClass::from_height(2160), SizeClass::Uhd4k);
        assert_eq!(SizeClass::from_height(4320), SizeClass::Uhd4k);
    }

    #[test]
    fn h264_ranges_match_the_brief_and_handbrake() {
        assert_eq!(suggested_range(VideoCodec::H264, SizeClass::Sd), (18, 22));
        assert_eq!(suggested_range(VideoCodec::H264, SizeClass::Hd720), (19, 23));
        assert_eq!(suggested_range(VideoCodec::H264, SizeClass::Hd1080), (20, 24));
        assert_eq!(suggested_range(VideoCodec::H264, SizeClass::Uhd4k), (22, 28));
    }

    #[test]
    fn hevc_uses_the_same_handbrake_x264_x265_table() {
        for class in [SizeClass::Sd, SizeClass::Hd720, SizeClass::Hd1080, SizeClass::Uhd4k] {
            assert_eq!(suggested_range(VideoCodec::Hevc, class), suggested_range(VideoCodec::H264, class));
        }
    }

    #[test]
    fn av1_ranges_match_handbrake_svt_av1() {
        assert_eq!(suggested_range(VideoCodec::Av1, SizeClass::Sd), (22, 32));
        assert_eq!(suggested_range(VideoCodec::Av1, SizeClass::Hd720), (25, 35));
        assert_eq!(suggested_range(VideoCodec::Av1, SizeClass::Hd1080), (25, 35));
        assert_eq!(suggested_range(VideoCodec::Av1, SizeClass::Uhd4k), (25, 40));
    }

    #[test]
    fn popup_explains_crf_and_does_not_change_values() {
        let t = popup_text(VideoCodec::H264, Some(1080));
        assert!(t.contains("lower number means better quality"));
        assert!(t.contains("20–24"));
        assert!(t.contains("1080p"));
        assert!(t.contains("Your CRF value is not changed."));
        let t = popup_text(VideoCodec::Av1, Some(2160));
        assert!(t.contains("25–40"));
        let t = popup_text(VideoCodec::Hevc, None);
        assert!(t.contains("Add a video"));
        assert!(t.contains("Your CRF value is not changed."));
        let t = popup_text(VideoCodec::H264, Some(1440));
        assert!(t.contains("closest official guide"));
        assert!(t.contains("20–24"));
    }
}
