//! Grab one picture from a video (or read an image) with the app's FFmpeg, for the crop and
//! watermark previews. Pure argument building is unit-tested; [`grab`] runs FFmpeg and decodes
//! the PNG it writes to stdout. Callers run it on a background thread.

use std::path::Path;

use crate::util::fmt_num;

/// One decoded RGBA picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Time of `fraction` (0–1) through a `duration_s` long file, in seconds (0 when unknown).
pub fn time_at(duration_s: f64, fraction: f64) -> f64 {
    if !duration_s.is_finite() || duration_s <= 0.0 {
        return 0.0;
    }
    (duration_s * fraction.clamp(0.0, 1.0) * 1000.0).round() / 1000.0
}

/// `-v error -ss T -i IN -frames:v 1 -an -sn -dn -vf scale=W:-2 -pix_fmt rgba -f image2pipe -c:v png -`
/// (`at_s` = None for still images; `width` = None keeps the size).
pub fn grab_args(input: &Path, at_s: Option<f64>, width: Option<u32>) -> Vec<String> {
    let mut a: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-v".into(), "error".into()];
    if let Some(t) = at_s.filter(|t| *t > 0.0) {
        a.push("-ss".into());
        a.push(fmt_num(t));
    }
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());
    a.extend(["-frames:v", "1", "-an", "-sn", "-dn"].map(String::from));
    if let Some(w) = width.filter(|w| *w > 0) {
        a.push("-vf".into());
        a.push(format!("scale={w}:-2"));
    }
    a.extend(["-pix_fmt", "rgba", "-f", "image2pipe", "-c:v", "png", "-"].map(String::from));
    a
}

/// Run FFmpeg and decode the picture. Falls back to the first frame when seeking finds none.
pub fn grab(ffmpeg: &Path, input: &Path, at_s: Option<f64>, width: Option<u32>) -> Result<Frame, String> {
    match grab_once(ffmpeg, input, at_s, width) {
        Err(_) if at_s.map(|t| t > 0.0).unwrap_or(false) => grab_once(ffmpeg, input, None, width),
        r => r,
    }
}

fn grab_once(ffmpeg: &Path, input: &Path, at_s: Option<f64>, width: Option<u32>) -> Result<Frame, String> {
    let out = super::hidden_command(ffmpeg)
        .args(grab_args(input, at_s, width))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not run FFmpeg: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no picture returned").trim().to_string();
        return Err(format!("No preview: {line}"));
    }
    decode_png(&out.stdout)
}

pub fn decode_png(bytes: &[u8]) -> Result<Frame, String> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).map_err(|e| format!("No preview: {e}"))?;
    let rgba = img.to_rgba8();
    Ok(Frame { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_times() {
        assert_eq!(time_at(100.0, 0.5), 50.0);
        assert_eq!(time_at(0.0, 0.5), 0.0);
        assert_eq!(time_at(f64::NAN, 0.5), 0.0);
        assert_eq!(time_at(10.0, 0.333), 3.33);
        assert_eq!(
            grab_args(Path::new("/v/a b.mkv"), Some(12.5), Some(480)).join(" "),
            "-hide_banner -nostdin -v error -ss 12.5 -i /v/a b.mkv -frames:v 1 -an -sn -dn -vf scale=480:-2 -pix_fmt rgba -f image2pipe -c:v png -"
        );
        assert_eq!(
            grab_args(Path::new("/i/logo.png"), None, None).join(" "),
            "-hide_banner -nostdin -v error -i /i/logo.png -frames:v 1 -an -sn -dn -pix_fmt rgba -f image2pipe -c:v png -"
        );
    }
}
