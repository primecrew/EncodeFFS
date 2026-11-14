//! Remux MKV → MP4: warnings for incompatible tracks, command unchanged.
//! Skips when ffmpeg is not on PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

use ffenc::cmd::remux::{self, RemuxJob};
use ffenc::ffmpeg::probe;

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

fn ff(ffmpeg: &Path, args: &[&str], dir: &Path) {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "ffmpeg failed {:?}\n{}", args, String::from_utf8_lossy(&out.stderr));
}

#[test]
fn incompatible_truehd_is_warned_and_command_is_unchanged() {
    let Some(ffmpeg) = which("ffmpeg") else { return };
    let Some(ffprobe) = which("ffprobe") else { return };
    let dir = std::env::temp_dir().join(format!("ffenc_remux_compat_{}", ffenc::util::unique_id()));
    std::fs::create_dir_all(&dir).unwrap();

    ff(&ffmpeg, &["-f", "lavfi", "-i", "color=c=black:s=320x240:d=0.4:r=25", "-c:v", "libx264", "-pix_fmt", "yuv420p", "vid.mkv"], &dir);
    // truehd encoder is experimental
    ff(&ffmpeg, &["-f", "lavfi", "-i", "sine=f=440:d=0.4:r=48000", "-strict", "-2", "-c:a", "truehd", "truehd.thd"], &dir);
    ff(&ffmpeg, &["-i", "vid.mkv", "-i", "truehd.thd", "-map", "0:v", "-map", "1:a", "-c", "copy", "truehd.mkv"], &dir);

    let src = dir.join("truehd.mkv");
    let info = probe::probe(&ffprobe, &src).unwrap();
    assert_eq!(info.of_type("audio").next().unwrap().codec_name, "truehd");
    let w = remux::copy_warnings(&info);
    assert_eq!(
        w,
        ["Track 1 (TrueHD) can't be copied into an MP4. Remux to MKV, or re-encode that audio."]
    );

    let job = RemuxJob { source: src.clone(), dest: dir.join("truehd.mp4"), temp: dir.join(".remux_1.mp4") };
    let args = remux::args(&job);
    assert_eq!(
        args.join(" "),
        format!("-nostdin -n -i {} -map 0 -c copy {}", src.display(), job.temp.display())
    );

    // The command is unchanged, so ffmpeg still rejects TrueHD without -strict -2.
    let out = Command::new(&ffmpeg).args(&args).output().unwrap();
    assert!(!out.status.success(), "truehd remux should fail with the stock command");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("truehd") && (err.contains("experimental") || err.contains("strict")), "{err}");
    assert!(!job.dest.exists());

    // pcm_s24le: ffmpeg will mux it; we only warn about player support; command still succeeds.
    ff(&ffmpeg, &["-f", "lavfi", "-i", "sine=f=440:d=0.4:r=48000", "-c:a", "pcm_s24le", "pcm24.wav"], &dir);
    ff(&ffmpeg, &["-i", "vid.mkv", "-i", "pcm24.wav", "-map", "0:v", "-map", "1:a", "-c", "copy", "pcm24.mkv"], &dir);
    let pcm = dir.join("pcm24.mkv");
    let info = probe::probe(&ffprobe, &pcm).unwrap();
    assert_eq!(
        remux::copy_warnings(&info),
        ["Track 1 (PCM s24le) can be stored in MP4, but many players will not play it. Remux to MKV for wider support."]
    );
    let job = RemuxJob { source: pcm.clone(), dest: dir.join("pcm24.mp4"), temp: dir.join(".remux_pcm.mp4") };
    assert_eq!(
        remux::args(&job).join(" "),
        format!("-nostdin -n -i {} -map 0 -c copy {}", pcm.display(), job.temp.display())
    );
    let out = Command::new(&ffmpeg).args(remux::args(&job)).output().unwrap();
    assert!(out.status.success(), "pcm_s24le remux should succeed:\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(job.temp.is_file());

    let _ = std::fs::remove_dir_all(&dir);
}
