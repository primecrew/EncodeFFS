//! 1.1 crop + watermark, end to end with the system FFmpeg: build the command with the real
//! builder, run it, and check size, length, placement and opacity of the result. Also checks the
//! preview frame grab. Skips (passes) when ffmpeg is not on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffenc::cmd::common::*;
use ffenc::cmd::picture::{CropSettings, PictureSettings, WatermarkSettings};
use ffenc::cmd::reencode::{self, ReencodeParams};
use ffenc::ffmpeg::framegrab;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn ff(ffmpeg: &Path, args: &[&str]) {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg failed: {args:?}\n{}", String::from_utf8_lossy(&out.stderr));
}

fn probe(ffprobe: &Path, f: &Path, entry: &str) -> String {
    let out = Command::new(ffprobe).args(["-v", "error", "-select_streams", "v:0", "-show_entries", entry, "-of", "default=nw=1:nk=1"]).arg(f).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fmt_duration(ffprobe: &Path, f: &Path) -> f64 {
    let out = Command::new(ffprobe).args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(f).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

fn px(f: &framegrab::Frame, x: u32, y: u32) -> [i32; 3] {
    let i = ((y * f.width + x) * 4) as usize;
    [f.rgba[i] as i32, f.rgba[i + 1] as i32, f.rgba[i + 2] as i32]
}

fn near(a: [i32; 3], b: [i32; 3], tol: i32) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tol)
}

fn params(input: &Path, output: &Path, pic: &PictureSettings, img: Option<(u32, u32)>) -> ReencodeParams {
    let (ops, warnings) = pic.resolve(1280, 720, img).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    ReencodeParams {
        input: input.into(),
        output: output.into(),
        duration_s: 4.0,
        trim_start: 0.0,
        trim_end: 0.0,
        passthrough: Passthrough::None,
        video: VideoEncode { crf: 4, preset: "ultrafast".into(), ..Default::default() },
        audio: AudioEncode { codec: ffenc::cmd::codecs::AudioCodec::Copy, ..Default::default() },
        range_convert: false,
        resize: None,
        deinterlace: false,
        chapters: None,
        faststart: false,
        subtitle_streams: vec![],
        has_attachments: false,
        picture: ops,
    }
}

#[test]
fn crop_and_watermark_encode() {
    let (Some(ffmpeg), Some(ffprobe)) = (which("ffmpeg"), which("ffprobe")) else {
        eprintln!("ffmpeg not on PATH – skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("ffenc_pic_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("clip.mp4");
    let logo = dir.join("logo.png");
    // flat grey picture so blending is easy to measure; sine audio
    ff(&ffmpeg, &["-f", "lavfi", "-i", "color=c=0x606060:s=1280x720:r=25:d=4", "-f", "lavfi", "-i", "sine=f=440:d=4", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", clip.to_str().unwrap()]);
    // 200x100 logo: left half solid red, right half fully transparent
    ff(&ffmpeg, &["-f", "lavfi", "-i", "color=c=red:s=200x100,format=rgba,geq=r=255:g=0:b=0:a='if(lt(X,100),255,0)'", "-frames:v", "1", logo.to_str().unwrap()]);

    // preview grab (the shared function): 50 % through, half size
    let g = framegrab::grab(&ffmpeg, &clip, Some(framegrab::time_at(4.0, 0.5)), Some(640)).unwrap();
    assert_eq!((g.width, g.height), (640, 360));
    let li = framegrab::grab(&ffmpeg, &logo, None, None).unwrap();
    assert_eq!((li.width, li.height), (200, 100));
    assert_eq!(li.rgba[3], 255);
    assert_eq!(li.rgba[(150 * 4 + 3) as usize], 0, "PNG transparency kept");

    // crop 60 px top and bottom, 40 left / right -> 1200x600; watermark 25 % wide, at x=10 %, y=20 %, 50 % opacity
    let pic = PictureSettings {
        crop: CropSettings { enabled: true, top: 60, right: 40, bottom: 60, left: 40 },
        watermark: WatermarkSettings { enabled: true, image: logo.to_string_lossy().into(), x: 0.1, y: 0.2, w: 0.25, opacity_pct: 50, ..Default::default() },
    };
    let out = dir.join("out.mp4");
    let pr = params(&clip, &out, &pic, Some((200, 100)));
    let cmd = reencode::build(&pr).unwrap();
    eprintln!("ffmpeg {}", cmd.args.join(" "));
    let run = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error"]).args(&cmd.args).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));

    // size = cropped, length = the video's, audio kept
    assert_eq!(probe(&ffprobe, &out, "stream=width"), "1200");
    assert_eq!(probe(&ffprobe, &out, "stream=height"), "600");
    let d = fmt_duration(&ffprobe, &out);
    assert!((d - 4.0).abs() < 0.15, "duration {d}");
    let a = Command::new(&ffprobe).args(["-v", "error", "-select_streams", "a", "-show_entries", "stream=codec_name", "-of", "csv=p=0"]).arg(&out).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&a.stdout).trim(), "aac");

    // watermark: 300x150 at (120, 120) of the 1200x600 frame; red left half at 50 %
    for t in [0.2, 3.8] {
        let f = framegrab::grab(&ffmpeg, &out, Some(t), None).unwrap();
        assert_eq!((f.width, f.height), (1200, 600));
        let bg = px(&f, 1000, 500);
        assert!(near(bg, [0x60, 0x60, 0x60], 6), "background {bg:?}");
        let red = px(&f, 120 + 75, 120 + 75);
        // 50 % red over grey 96: R ≈ 175, G ≈ B ≈ 48
        assert!(near(red, [175, 48, 48], 18), "half-transparent red at {t}s: {red:?}");
        let clear = px(&f, 120 + 225, 120 + 75);
        assert!(near(clear, bg, 6), "transparent half of the logo must show the video: {clear:?}");
        let left_of = px(&f, 110, 190);
        assert!(near(left_of, bg, 6), "just left of the watermark: {left_of:?}");
    }

    // everything off = the old command and a plain encode
    let off = params(&clip, &out, &PictureSettings::default(), None);
    let mut old = off.clone();
    old.picture = Default::default();
    assert_eq!(reencode::build(&off).unwrap(), reencode::build(&old).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}
