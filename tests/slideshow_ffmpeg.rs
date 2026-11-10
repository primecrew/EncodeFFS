//! Real ffmpeg smoke: 3 stills → slideshow with xfade and without. Skips if ffmpeg missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffenc::cmd::common::VideoEncode;
use ffenc::cmd::slideshow::*;
use ffenc::ffmpeg::version::{has_filter, check_ffmpeg};

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn make_png(ffmpeg: &Path, out: &Path, color: &str) {
    assert!(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("color=c={color}:s=320x240:d=0.1"), "-frames:v", "1"])
        .arg(out)
        .status()
        .unwrap()
        .success());
}

fn probe_dur(ffprobe: &Path, f: &Path) -> f64 {
    let o = Command::new(ffprobe).args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(f).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
}

#[test]
fn encode_with_and_without_xfade() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else { return };
    let has_xfade = check_ffmpeg(&ffmpeg).map(|c| c.has_xfade).unwrap_or_else(|_| {
        let out = Command::new(&ffmpeg).args(["-hide_banner", "-filters"]).output().ok();
        out.map(|o| has_filter(&String::from_utf8_lossy(&o.stdout), "xfade")).unwrap_or(false)
    });
    let dir = std::env::temp_dir().join(format!("ffenc_ss_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let imgs: Vec<_> = ["red", "green", "blue"].iter().enumerate().map(|(i, c)| {
        let p = dir.join(format!("{i}.png"));
        make_png(&ffmpeg, &p, c);
        ImageInput { path: p, width: 320, height: 240 }
    }).collect();

    let run = |name: &str, transition: Transition, has: bool| {
        let out = dir.join(name);
        let pr = SlideshowParams {
            images: imgs.clone(),
            audio: None,
            audio_duration_s: None,
            output: out.clone(),
            duration_mode: DurationMode::PerImage,
            per_image_s: 2.0,
            fixed_total_s: 60.0,
            transition,
            fade_s: 0.5,
            has_xfade: has,
            size: OutputSize::S640x480,
            small: SmallImage::BlackBorder,
            fps: 30,
            video: VideoEncode { crf: 28, preset: "ultrafast".into(), ..Default::default() },
            audio_bitrate_k: 128,
            faststart: true,
        };
        let cmd = build(&pr).unwrap();
        let t = timing(&pr).unwrap();
        let st = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&cmd.args).output().unwrap();
        assert!(st.status.success(), "{name}: {}", String::from_utf8_lossy(&st.stderr));
        let dur = probe_dur(&ffprobe, &out);
        assert!((dur - t.total_s).abs() < 0.35, "{name}: got {dur}, want {}", t.total_s);
        out
    };

    // hard cuts: total = 3*2 = 6
    run("cuts.mp4", Transition::None, true);
    if has_xfade {
        // additive fade 0.5: total = 6 + 2*0.5 = 7
        run("fade.mp4", Transition::Fade, true);

        // 3 images, 5 s each, 2 s fade → 3*5 + 2*2 = 19 s
        let out19 = dir.join("fade19.mp4");
        let pr = SlideshowParams {
            images: imgs.clone(),
            audio: None,
            audio_duration_s: None,
            output: out19.clone(),
            duration_mode: DurationMode::PerImage,
            per_image_s: 5.0,
            fixed_total_s: 60.0,
            transition: Transition::Fade,
            fade_s: 2.0,
            has_xfade: true,
            size: OutputSize::S640x480,
            small: SmallImage::BlackBorder,
            fps: 30,
            video: VideoEncode { crf: 28, preset: "ultrafast".into(), ..Default::default() },
            audio_bitrate_k: 128,
            faststart: true,
        };
        let t = timing(&pr).unwrap();
        assert!((t.total_s - 19.0).abs() < 1e-9, "timing should be 19 s, got {}", t.total_s);
        let cmd = build(&pr).unwrap();
        let st = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&cmd.args).output().unwrap();
        assert!(st.status.success(), "fade19: {}", String::from_utf8_lossy(&st.stderr));
        let dur = probe_dur(&ffprobe, &out19);
        assert!((dur - 19.0).abs() < 0.35, "3 images × 5 s + 2 fades × 2 s: got {dur}, want 19");
    } else {
        // force fallback path: transition time is not added (3*2 = 6)
        let _ = run("fallback.mp4", Transition::Fade, false);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
