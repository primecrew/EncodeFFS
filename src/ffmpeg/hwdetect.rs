//! GPU encoder detection (1.1): list the encoders, then prove each one with a ~1 s test encode.
//! Runs on a background thread (the UI never waits). Only runs the given ffmpeg; nothing is
//! written anywhere except the settings file (by the caller).

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::cmd::codecs::VideoCodec;
use crate::cmd::hwenc::{self, HwDetect, HwTest, HwVendor};

/// Longest a single probe may take before it is stopped (a hung driver must not hang the check).
const TIMEOUT: Duration = Duration::from_secs(20);

/// Run `program args` with a timeout. Ok(stdout) on exit code 0, Err(last stderr line) otherwise.
fn run(program: &Path, args: &[String]) -> Result<String, String> {
    let mut child = super::hidden_command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    // read both pipes on threads so a chatty process can't block on a full pipe
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = so.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = se.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };
    let out = t_out.join().unwrap_or_default();
    let err = t_err.join().unwrap_or_default();
    match status {
        Some(st) if st.success() => Ok(out),
        // the first line names the cause ("Cannot load nvcuda.dll", "No device available", …)
        Some(_) => Err(err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("test encode failed").to_string()),
        None => Err("timed out".into()),
    }
}

/// GPU name, when it is cheap to get (nvidia-smi for NVIDIA; the Windows video controller list
/// otherwise). None = the caller names the vendor instead.
pub fn gpu_name(vendor: HwVendor) -> Option<String> {
    if vendor == HwVendor::Nvidia {
        if let Ok(out) = run(Path::new("nvidia-smi"), &["--query-gpu=name".into(), "--format=csv,noheader".into()]) {
            if let Some(n) = out.lines().map(str::trim).find(|l| !l.is_empty()) {
                return Some(n.to_string());
            }
        }
    }
    #[cfg(windows)]
    {
        let args: Vec<String> = ["-NoProfile", "-NonInteractive", "-Command", "(Get-CimInstance Win32_VideoController).Name"].iter().map(|s| s.to_string()).collect();
        if let Ok(out) = run(Path::new("powershell"), &args) {
            return pick_gpu_name(&out, vendor);
        }
    }
    None
}

/// From a list of adapter names (one per line), the first that belongs to `vendor`.
pub fn pick_gpu_name(list: &str, vendor: HwVendor) -> Option<String> {
    let keys: &[&str] = match vendor {
        HwVendor::Nvidia => &["nvidia"],
        HwVendor::Intel => &["intel"],
        HwVendor::Amd => &["amd", "radeon"],
    };
    list.lines().map(str::trim).find(|l| {
        let low = l.to_lowercase();
        keys.iter().any(|k| low.contains(k))
    }).map(str::to_string)
}

/// List + test every GPU encoder of `program`. Never fails: problems end up as failed tests.
pub fn detect(program: &Path) -> HwDetect {
    let mut d = HwDetect {
        program: program.to_string_lossy().into_owned(),
        checked: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_secs()).unwrap_or(0),
        ..Default::default()
    };
    let list = match run(program, &["-hide_banner".into(), "-encoders".into()]) {
        Ok(l) => l,
        Err(_) => return d,
    };
    for (codec, vendor) in hwenc::parse_encoders(&list) {
        let (ok, error) = match run(program, &hwenc::test_args(codec, vendor, false)) {
            Ok(_) => (true, String::new()),
            Err(e) => (false, e),
        };
        let ok_10bit = ok && codec != VideoCodec::H264 && run(program, &hwenc::test_args(codec, vendor, true)).is_ok();
        d.tests.push(HwTest { codec, vendor, ok, ok_10bit, error });
    }
    if let Some(v) = d.vendor() {
        d.gpu_name = gpu_name(v);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_name_from_adapter_list() {
        let list = "Microsoft Basic Display Adapter\r\nIntel(R) UHD Graphics 770\r\nNVIDIA GeForce RTX 4070\r\n";
        assert_eq!(pick_gpu_name(list, HwVendor::Nvidia).as_deref(), Some("NVIDIA GeForce RTX 4070"));
        assert_eq!(pick_gpu_name(list, HwVendor::Intel).as_deref(), Some("Intel(R) UHD Graphics 770"));
        assert_eq!(pick_gpu_name("AMD Radeon RX 7800 XT\n", HwVendor::Amd).as_deref(), Some("AMD Radeon RX 7800 XT"));
        assert_eq!(pick_gpu_name(list, HwVendor::Amd), None);
    }

    #[test]
    fn detect_on_a_missing_program_is_empty() {
        let d = detect(Path::new("/nonexistent/ffmpeg"));
        assert!(d.tests.is_empty());
        assert_eq!(hwenc::summary(Some(&d)), "No GPU encoder found. Encoding uses the CPU.");
    }
}
