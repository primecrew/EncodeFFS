//! Download -> verify -> extract -> test -> swap in (with backup) -> roll back.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::sources::{ArchiveKind, AvailableBuild};

/// Binaries we take from a build. The first two are required.
pub const WANTED: [&str; 3] = ["ffmpeg.exe", "ffprobe.exe", "ffplay.exe"];
pub const REQUIRED: [&str; 2] = ["ffmpeg.exe", "ffprobe.exe"];

pub fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .user_agent(&format!("FFmpegEncoder/{}", crate::APP_VERSION))
        .build()
}

pub fn fetch_text(url: &str) -> Result<String, String> {
    let resp = http_agent()
        .get(url)
        .set("Accept", "application/vnd.github+json, text/plain, */*")
        .call()
        .map_err(|e| format!("Request to {url} failed: {e}"))?;
    resp.into_string().map_err(|e| format!("Reading {url} failed: {e}"))
}

/// Stream `url` to `dest`, hashing as we go. Returns the SHA-256 (lower-case hex).
pub fn download(url: &str, dest: &Path, progress: &dyn Fn(u64, Option<u64>), cancel: &AtomicBool) -> Result<String, String> {
    let resp = http_agent().get(url).call().map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut file = File::create(dest).map_err(|e| format!("Cannot write {}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Download cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("Write failed: {e}"))?;
        done += n as u64;
        progress(done, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    if let Some(t) = total {
        if done != t {
            return Err(format!("Download incomplete: {done} of {t} bytes"));
        }
    }
    Ok(hex(&hasher.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| e.to_string())?;
    Ok(hex(&h.finalize()))
}

/// Ok(true) = verified, Ok(false) = no checksum published, Err = mismatch.
pub fn verify_sha256(actual: &str, expected: Option<&str>) -> Result<bool, String> {
    match expected {
        None => Ok(false),
        Some(e) if e.eq_ignore_ascii_case(actual) => Ok(true),
        Some(e) => Err(format!("Checksum mismatch: expected {e}, got {actual}. The download was discarded.")),
    }
}

fn base_name(entry: &str) -> &str {
    entry.rsplit(['/', '\\']).next().unwrap_or(entry)
}

/// Extract the wanted binaries (matched by file name, any folder depth) into `out_dir`.
/// Only bare file names are used for output paths, so archive paths can't escape `out_dir`.
pub fn extract_binaries(archive: &Path, kind: ArchiveKind, out_dir: &Path, wanted: &[&str]) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut got: Vec<String> = Vec::new();
    match kind {
        ArchiveKind::Zip => {
            let f = File::open(archive).map_err(|e| e.to_string())?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| format!("Not a valid zip: {e}"))?;
            for i in 0..z.len() {
                let mut entry = z.by_index(i).map_err(|e| e.to_string())?;
                if entry.is_dir() {
                    continue;
                }
                let name = base_name(entry.name()).to_string();
                if wanted.iter().any(|w| w.eq_ignore_ascii_case(&name)) && !got.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
                    let mut out = File::create(out_dir.join(&name)).map_err(|e| e.to_string())?;
                    std::io::copy(&mut entry, &mut out).map_err(|e| format!("Extracting {name} failed: {e}"))?;
                    got.push(name);
                }
            }
        }
        ArchiveKind::SevenZ => {
            let mut r = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
                .map_err(|e| format!("Not a valid 7z: {e}"))?;
            let mut err: Option<String> = None;
            r.for_each_entries(|entry, reader| {
                if entry.is_directory {
                    return Ok(true);
                }
                let name = base_name(&entry.name).to_string();
                if wanted.iter().any(|w| w.eq_ignore_ascii_case(&name)) && !got.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
                    match File::create(out_dir.join(&name)) {
                        Ok(mut out) => {
                            if let Err(e) = std::io::copy(reader, &mut out) {
                                err = Some(format!("Extracting {name} failed: {e}"));
                                return Ok(false);
                            }
                            got.push(name);
                        }
                        Err(e) => {
                            err = Some(e.to_string());
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            })
            .map_err(|e| format!("7z extraction failed: {e}"))?;
            if let Some(e) = err {
                return Err(e);
            }
        }
    }
    for r in REQUIRED.iter().filter(|r| wanted.contains(r)) {
        if !got.iter().any(|g| g.eq_ignore_ascii_case(r)) {
            return Err(format!("The archive does not contain {r}"));
        }
    }
    Ok(got)
}

/// Backup folder for a bin folder: `<bin>.previous` next to it.
pub fn backup_dir_for(target_bin: &Path) -> PathBuf {
    let name = target_bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "bin".into());
    target_bin.with_file_name(format!("{name}.previous"))
}

pub fn has_backup(target_bin: &Path) -> bool {
    REQUIRED.iter().all(|n| backup_dir_for(target_bin).join(n).is_file())
}

/// Can we create files in `dir` (or create `dir`)?
pub fn can_write(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let p = dir.join(format!(".ffenc_w_{}", crate::util::unique_id()));
    let ok = std::fs::write(&p, b"x").is_ok();
    let _ = std::fs::remove_file(&p);
    ok
}

/// Swap new binaries into `target_bin`. Existing binaries are moved to the backup
/// folder first; if any step fails, everything is moved back. Only the named
/// files are touched, so other files in a user's folder are left alone.
pub fn install_files(new_dir: &Path, target_bin: &Path, names: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(target_bin).map_err(|e| format!("Cannot create {}: {e}", target_bin.display()))?;
    let backup = backup_dir_for(target_bin);
    // Fresh backup generation (only once we know there is something to back up).
    let has_old = WANTED.iter().any(|n| target_bin.join(n).exists());
    if has_old {
        if backup.exists() {
            std::fs::remove_dir_all(&backup).map_err(|e| format!("Cannot clear old backup {}: {e}", backup.display()))?;
        }
        std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    }
    let mut moved_old: Vec<&str> = Vec::new();
    let mut placed: Vec<&str> = Vec::new();
    let result = (|| -> Result<(), String> {
        // Move *all* known binaries away (so e.g. an old ffplay never mixes with a new ffmpeg).
        for n in WANTED {
            let cur = target_bin.join(n);
            if cur.exists() {
                std::fs::rename(&cur, backup.join(n)).map_err(|e| format!("Cannot move {} to backup (is it running?): {e}", cur.display()))?;
                moved_old.push(n);
            }
        }
        for n in names {
            let src = new_dir.join(n);
            let n_static: &str = WANTED.iter().find(|w| w.eq_ignore_ascii_case(n)).copied().unwrap_or("");
            std::fs::rename(&src, target_bin.join(n)).map_err(|e| format!("Cannot place {}: {e}", n))?;
            placed.push(n_static);
        }
        Ok(())
    })();
    if let Err(e) = result {
        for n in &placed {
            let _ = std::fs::remove_file(target_bin.join(n));
        }
        for n in &moved_old {
            let _ = std::fs::rename(backup.join(n), target_bin.join(n));
        }
        return Err(format!("{e}. The previous FFmpeg was restored."));
    }
    Ok(())
}

/// Swap the current binaries with the backup ones (running it twice undoes it).
pub fn rollback(target_bin: &Path) -> Result<(), String> {
    let backup = backup_dir_for(target_bin);
    if !has_backup(target_bin) {
        return Err(format!("No previous version found in {}", backup.display()));
    }
    let tmp = target_bin.with_file_name(format!(".ffenc_swap_{}", crate::util::unique_id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mv = |a: &Path, b: &Path| -> Result<(), String> {
        if a.exists() {
            std::fs::rename(a, b).map_err(|e| format!("Cannot move {}: {e}", a.display()))
        } else {
            Ok(())
        }
    };
    let r = (|| -> Result<(), String> {
        for n in WANTED {
            mv(&target_bin.join(n), &tmp.join(n))?;
        }
        for n in WANTED {
            mv(&backup.join(n), &target_bin.join(n))?;
        }
        for n in WANTED {
            mv(&tmp.join(n), &backup.join(n))?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir(&tmp);
    r
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Downloading { done: u64, total: Option<u64> },
    Verifying,
    Extracting,
    Testing,
    Installing,
}

/// Full update into `target_bin`. Staging happens in a sibling folder so the
/// final moves are same-volume renames. Returns a short report.
pub fn run_update(build: &AvailableBuild, target_bin: &Path, stage: &dyn Fn(Stage), cancel: &AtomicBool) -> Result<String, String> {
    let parent = target_bin.parent().ok_or("Invalid target folder")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    let staging = parent.join(format!(".ffmpeg-update-{}", crate::util::unique_id()));
    std::fs::create_dir_all(&staging).map_err(|e| format!("Cannot write to {}: {e}", parent.display()))?;
    let res = (|| -> Result<String, String> {
        let archive = staging.join(crate::util::safe_name(&build.asset_name));
        let actual = download(&build.url, &archive, &|d, t| stage(Stage::Downloading { done: d, total: t }), cancel)?;
        stage(Stage::Verifying);
        let verified = verify_sha256(&actual, build.sha256.as_deref())?;
        stage(Stage::Extracting);
        let new_dir = staging.join("new");
        let names = extract_binaries(&archive, build.archive, &new_dir, &WANTED)?;
        let _ = std::fs::remove_file(&archive);
        stage(Stage::Testing);
        #[cfg(windows)]
        {
            let chk = crate::ffmpeg::version::check_ffmpeg(&new_dir.join("ffmpeg.exe"))?;
            if !chk.has_libx264 || !chk.has_aac {
                return Err("The downloaded build lacks libx264 or aac; not installed.".into());
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Update cancelled".into());
        }
        stage(Stage::Installing);
        install_files(&new_dir, target_bin, &names)?;
        let backup = if has_backup(target_bin) {
            format!(" Previous version kept in {}.", backup_dir_for(target_bin).display())
        } else {
            String::new()
        };
        Ok(format!(
            "Installed {} into {} ({}).{}",
            build.label,
            target_bin.display(),
            if verified { format!("SHA-256 verified: {actual}") } else { format!("no checksum published; SHA-256 {actual}") },
            backup
        ))
    })();
    let _ = std::fs::remove_dir_all(&staging);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("upd_{}", crate::util::unique_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        z.add_directory("ffmpeg-x/bin/", opts).unwrap();
        for (n, data) in files {
            z.start_file(*n, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn checksum() {
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(verify_sha256("aa", Some("AA")), Ok(true));
        assert_eq!(verify_sha256("aa", None), Ok(false));
        assert!(verify_sha256("aa", Some("bb")).is_err());
    }

    #[test]
    fn extract_and_install_and_rollback() {
        let d = tmp();
        let zip_path = d.join("b.zip");
        make_zip(&zip_path, &[
            ("ffmpeg-x/bin/ffmpeg.exe", b"new-ffmpeg"),
            ("ffmpeg-x/bin/ffprobe.exe", b"new-ffprobe"),
            ("ffmpeg-x/doc/readme.txt", b"doc"),
            ("../../evil/ffplay.exe", b"new-ffplay"),
        ]);
        let new_dir = d.join("new");
        let names = extract_binaries(&zip_path, ArchiveKind::Zip, &new_dir, &WANTED).unwrap();
        assert_eq!(names.len(), 3);
        assert!(new_dir.join("ffplay.exe").is_file()); // zip-slip path flattened
        assert!(!d.parent().unwrap().join("evil").exists());

        // existing install with an extra user file
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("ffmpeg.exe"), b"old-ffmpeg").unwrap();
        std::fs::write(bin.join("ffprobe.exe"), b"old-ffprobe").unwrap();
        std::fs::write(bin.join("other.dll"), b"keep").unwrap();
        install_files(&new_dir, &bin, &names).unwrap();
        assert_eq!(std::fs::read(bin.join("ffmpeg.exe")).unwrap(), b"new-ffmpeg");
        assert_eq!(std::fs::read(bin.join("other.dll")).unwrap(), b"keep");
        assert!(has_backup(&bin));
        assert_eq!(std::fs::read(backup_dir_for(&bin).join("ffmpeg.exe")).unwrap(), b"old-ffmpeg");

        rollback(&bin).unwrap();
        assert_eq!(std::fs::read(bin.join("ffmpeg.exe")).unwrap(), b"old-ffmpeg");
        assert!(!bin.join("ffplay.exe").exists()); // old install had no ffplay
        rollback(&bin).unwrap(); // and forward again
        assert_eq!(std::fs::read(bin.join("ffmpeg.exe")).unwrap(), b"new-ffmpeg");
        assert!(bin.join("ffplay.exe").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_required_binary_fails() {
        let d = tmp();
        let zip_path = d.join("b.zip");
        make_zip(&zip_path, &[("bin/ffmpeg.exe", b"x")]);
        assert!(extract_binaries(&zip_path, ArchiveKind::Zip, &d.join("o"), &WANTED).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failed_install_restores_previous() {
        let d = tmp();
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("ffmpeg.exe"), b"old").unwrap();
        std::fs::write(bin.join("ffprobe.exe"), b"oldp").unwrap();
        let new_dir = d.join("new");
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(new_dir.join("ffmpeg.exe"), b"new").unwrap();
        // ffprobe.exe missing from new_dir -> second rename fails -> restore
        let r = install_files(&new_dir, &bin, &["ffmpeg.exe".into(), "ffprobe.exe".into()]);
        assert!(r.is_err());
        assert_eq!(std::fs::read(bin.join("ffmpeg.exe")).unwrap(), b"old");
        assert_eq!(std::fs::read(bin.join("ffprobe.exe")).unwrap(), b"oldp");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn backup_path() {
        assert_eq!(backup_dir_for(Path::new("/a/ffmpeg/bin")), PathBuf::from("/a/ffmpeg/bin.previous"));
    }

    /// Downloads a real build (~100-200 MB) into a temp dir and installs it there.
    /// Run with: cargo test --release -- --ignored live_download --nocapture
    #[test]
    #[ignore]
    fn live_download_btbn_and_gyan() {
        use super::super::{check, UpdateChannel, UpdateSource};
        let which = std::env::var("FFENC_LIVE").unwrap_or_else(|_| "btbn-stable".into());
        let (src, ch) = match which.as_str() {
            "gyan-stable" => (UpdateSource::Gyan, UpdateChannel::Stable),
            "gyan-latest" => (UpdateSource::Gyan, UpdateChannel::Latest),
            "btbn-latest" => (UpdateSource::BtbN, UpdateChannel::Latest),
            _ => (UpdateSource::BtbN, UpdateChannel::Stable),
        };
        let b = check(src, ch).unwrap();
        println!("{} -> {}", b.label, b.url);
        let d = tmp();
        let bin = d.join("ffmpeg").join("bin");
        let cancel = AtomicBool::new(false);
        let last = std::sync::Mutex::new(0u64);
        let report = run_update(&b, &bin, &|s| {
            if let Stage::Downloading { done, .. } = s {
                let mut l = last.lock().unwrap();
                if done - *l > 50_000_000 { *l = done; println!("  {} MB", done / 1_000_000); }
            } else { println!("  {s:?}"); }
        }, &cancel).unwrap();
        println!("{report}");
        assert!(bin.join("ffmpeg.exe").is_file() && bin.join("ffprobe.exe").is_file());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
