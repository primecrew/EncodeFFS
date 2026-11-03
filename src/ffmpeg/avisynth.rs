//! Can this FFmpeg read AviSynth scripts? Checks the avisynth demuxer, the build
//! configuration and where avisynth.dll is installed (and whether it is 32- or 64-bit).
//! Standard BtbN and gyan.dev Windows builds are configured with --enable-avisynth
//! and load avisynth.dll at run time, so the DLL decides whether it works.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
    Arm64,
    Other(u16),
}

impl Arch {
    pub fn label(self) -> String {
        match self {
            Arch::X86 => "32-bit".into(),
            Arch::X64 => "64-bit".into(),
            Arch::Arm64 => "ARM64".into(),
            Arch::Other(m) => format!("unknown type 0x{m:04x}"),
        }
    }
}

/// Machine type of a Windows PE file (first few KB are enough).
pub fn pe_machine(bytes: &[u8]) -> Option<Arch> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }
    let off = u32::from_le_bytes(bytes[0x3c..0x40].try_into().ok()?) as usize;
    if bytes.len() < off + 6 || &bytes[off..off + 4] != b"PE\0\0" {
        return None;
    }
    let m = u16::from_le_bytes([bytes[off + 4], bytes[off + 5]]);
    Some(match m {
        0x014c => Arch::X86,
        0x8664 => Arch::X64,
        0xaa64 => Arch::Arm64,
        o => Arch::Other(o),
    })
}

/// `ffmpeg -demuxers` lists " D  avisynth        AviSynth script".
pub fn has_demuxer(demuxers: &str) -> bool {
    demuxers.lines().any(|l| {
        let mut w = l.split_whitespace();
        matches!(w.next(), Some(f) if f.contains('D')) && w.next().map(|n| n.split(',').any(|x| x == "avisynth")).unwrap_or(false)
    })
}

pub fn configured(version: &str) -> bool {
    version.contains("--enable-avisynth")
}

#[derive(Debug, Clone, PartialEq)]
pub struct DllInfo {
    pub path: PathBuf,
    pub arch: Option<Arch>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// None = couldn't run ffmpeg.
    pub demuxer: Option<bool>,
    pub configured: bool,
    pub ffmpeg_arch: Option<Arch>,
    pub dlls: Vec<DllInfo>,
    /// True when the check can't look for the DLL (not Windows).
    pub not_windows: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Error,
}

/// Places a 64-bit (or 32-bit) process loads avisynth.dll from.
pub fn dll_candidates(ffmpeg_dir: &Path, system_root: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![ffmpeg_dir.join("avisynth.dll")];
    if let Some(r) = system_root {
        v.push(r.join("System32").join("avisynth.dll"));
        v.push(r.join("SysWOW64").join("avisynth.dll"));
    }
    v
}

fn read_head(p: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut b = vec![0u8; 4096];
    let n = f.read(&mut b).ok()?;
    b.truncate(n);
    Some(b)
}

/// Run the checks (spawns ffmpeg twice; call from a background thread).
pub fn check(ffmpeg: &Path) -> Status {
    let run = |args: &[&str]| -> Option<String> {
        let o = super::hidden_command(ffmpeg).args(args).stdin(std::process::Stdio::null()).output().ok()?;
        Some(String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
    };
    let demuxer = run(&["-hide_banner", "-demuxers"]).map(|t| has_demuxer(&t));
    let configured = run(&["-hide_banner", "-version"]).map(|t| configured(&t)).unwrap_or(false);
    let ffmpeg_arch = read_head(ffmpeg).and_then(|b| pe_machine(&b));
    let root = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("WINDIR")).map(PathBuf::from);
    let dir = ffmpeg.parent().unwrap_or(Path::new("."));
    let dlls = dll_candidates(dir, root.as_deref())
        .into_iter()
        .filter(|p| p.is_file())
        .map(|p| DllInfo { arch: read_head(&p).and_then(|b| pe_machine(&b)), path: p })
        .collect();
    Status { demuxer, configured, ffmpeg_arch, dlls, not_windows: !cfg!(windows) }
}

/// Plain-language findings, most important first.
pub fn advice(s: &Status) -> Vec<(Level, String)> {
    let mut v = vec![];
    match s.demuxer {
        None => v.push((Level::Warn, "Couldn't run FFmpeg to check AviSynth support.".to_string())),
        Some(false) => v.push((
            Level::Error,
            "This FFmpeg can't read AviSynth scripts (it has no “avisynth” demuxer). Install a standard Windows build with Help › Check for FFmpeg updates (BtbN or gyan.dev builds include it), or point File › Settings at one.".to_string(),
        )),
        Some(true) => v.push((Level::Ok, format!("FFmpeg can read AviSynth scripts{}.", if s.configured { " (built with --enable-avisynth)" } else { "" }))),
    }
    if s.not_windows {
        v.push((Level::Warn, "AviSynth+ and DebugMode FrameServer are Windows programs; the DLL check only works on Windows.".to_string()));
        return v;
    }
    let want = s.ffmpeg_arch.unwrap_or(Arch::X64);
    let matching: Vec<&DllInfo> = s.dlls.iter().filter(|d| d.arch == Some(want)).collect();
    if let Some(d) = matching.first() {
        v.push((Level::Ok, format!("AviSynth found: {} ({}, matches FFmpeg).", d.path.display(), want.label())));
    } else if let Some(d) = s.dlls.first() {
        v.push((
            Level::Error,
            format!(
                "Only a {} AviSynth was found ({}). This FFmpeg is {}, so it needs the {} AviSynth+ – install AviSynth+ (its installer adds both).",
                d.arch.map(|a| a.label()).unwrap_or_else(|| "unknown".into()),
                d.path.display(),
                want.label(),
                want.label()
            ),
        ));
    } else {
        v.push((
            Level::Error,
            format!("AviSynth doesn't seem to be installed (no avisynth.dll in Windows\\System32 or next to FFmpeg). Install AviSynth+ ({}) from github.com/AviSynth/AviSynthPlus/releases – DebugMode FrameServer needs it too.", want.label()),
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe(machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        b
    }

    #[test]
    fn pe_arch() {
        assert_eq!(pe_machine(&pe(0x8664)), Some(Arch::X64));
        assert_eq!(pe_machine(&pe(0x014c)), Some(Arch::X86));
        assert_eq!(pe_machine(&pe(0xaa64)), Some(Arch::Arm64));
        assert_eq!(pe_machine(b"#!/bin/sh"), None);
        let mut bad = pe(0x8664);
        bad[0x80] = b'X';
        assert_eq!(pe_machine(&bad), None);
    }

    #[test]
    fn demuxer_and_config() {
        let list = "File formats:\n D. = Demuxing supported\n --\n D  avi             AVI (Audio Video Interleaved)\n D  avisynth        AviSynth script\n D  mov,mp4,m4a     QuickTime / MOV\n";
        assert!(has_demuxer(list));
        assert!(!has_demuxer(" D  avi             AVI\n  E avisynth  fake muxer line\n"));
        assert!(configured("configuration: --enable-gpl --enable-avisynth --enable-libx264"));
        assert!(!configured("configuration: --enable-gpl"));
    }

    #[test]
    fn advice_cases() {
        let dir = Path::new("C:/ff/bin");
        let c = dll_candidates(dir, Some(Path::new("C:/Windows")));
        assert_eq!(c.len(), 3);
        assert!(c[1].ends_with("System32/avisynth.dll") && c[2].ends_with("SysWOW64/avisynth.dll"));
        let base = Status { demuxer: Some(true), configured: true, ffmpeg_arch: Some(Arch::X64), dlls: vec![], not_windows: false };
        let a = advice(&base);
        assert_eq!(a[0].0, Level::Ok);
        assert!(a[1].0 == Level::Error && a[1].1.contains("doesn't seem to be installed"));
        let only32 = Status { dlls: vec![DllInfo { path: "C:/Windows/SysWOW64/avisynth.dll".into(), arch: Some(Arch::X86) }], ..base.clone() };
        let a = advice(&only32);
        assert!(a[1].1.contains("Only a 32-bit") && a[1].1.contains("64-bit AviSynth+"), "{}", a[1].1);
        let good = Status { dlls: vec![DllInfo { path: "C:/Windows/System32/avisynth.dll".into(), arch: Some(Arch::X64) }], ..base.clone() };
        assert!(advice(&good).iter().all(|x| x.0 == Level::Ok));
        let nodemux = Status { demuxer: Some(false), ..good };
        assert_eq!(advice(&nodemux)[0].0, Level::Error);
    }
}
