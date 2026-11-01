//! Parsing of build-source metadata. Pure functions – tested against captured
//! responses and (in an ignored test) against the live network.

use serde::Deserialize;

use super::{UpdateChannel, UpdateSource};
use crate::ffmpeg::version::{parse_iso_date, parse_version, FfVersion};

pub const BTBN_API: &str = "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest";
pub const BTBN_RELEASES_PAGE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/tag/latest";
pub const GYAN_PAGE: &str = "https://www.gyan.dev/ffmpeg/builds/";
pub const GYAN_RELEASE_VERSION: &str = "https://www.gyan.dev/ffmpeg/builds/release-version";
pub const GYAN_RELEASE_ZIP: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
pub const GYAN_RELEASE_SHA: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256";
pub const GYAN_GIT_VERSION: &str = "https://www.gyan.dev/ffmpeg/builds/git-version";
pub const GYAN_GIT_7Z: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-git-essentials.7z";
pub const GYAN_GIT_SHA: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-git-essentials.7z.sha256";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZ,
}

/// A downloadable build.
#[derive(Debug, Clone, PartialEq)]
pub struct AvailableBuild {
    pub source: UpdateSource,
    pub channel: UpdateChannel,
    /// Human label, e.g. "n9.0 (2026-10-31)" or "9.0.2".
    pub label: String,
    pub version: FfVersion,
    pub url: String,
    pub asset_name: String,
    pub size: Option<u64>,
    /// Expected SHA-256 (lower-case hex) if the source publishes one.
    pub sha256: Option<String>,
    pub archive: ArchiveKind,
    /// FFmpeg changelog / commit list.
    pub notes_url: String,
    /// Build source page.
    pub page_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    published_at: Option<String>,
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
    digest: Option<String>,
    updated_at: Option<String>,
}

pub const BTBN_MASTER_ASSET: &str = "ffmpeg-master-latest-win64-gpl.zip";

/// "ffmpeg-n9.0-latest-win64-gpl-9.0.zip" -> Some((9, 0)). Rejects shared/lgpl/other OS variants.
pub fn parse_btbn_stable_name(name: &str) -> Option<(u32, u32)> {
    let rest = name.strip_prefix("ffmpeg-n")?.strip_suffix(".zip")?;
    let (a, b) = rest.split_once("-latest-win64-gpl-")?;
    if a != b {
        return None;
    }
    let (maj, min) = a.split_once('.')?;
    Some((maj.parse().ok()?, min.parse().ok()?))
}

fn sha_from_digest(d: &Option<String>) -> Option<String> {
    let d = d.as_ref()?;
    let hex = d.strip_prefix("sha256:")?;
    is_sha256(hex).then(|| hex.to_ascii_lowercase())
}

pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// First 64-hex token in a .sha256 file ("<hash>" or "<hash>  filename").
pub fn parse_sha256_text(text: &str) -> Option<String> {
    text.split_whitespace().find(|t| is_sha256(t)).map(|t| t.to_ascii_lowercase())
}

pub fn parse_btbn_release(json: &str, channel: UpdateChannel) -> Result<AvailableBuild, String> {
    let rel: GhRelease = serde_json::from_str(json).map_err(|e| format!("Unexpected GitHub API response: {e}"))?;
    let page_url = rel.html_url.clone().unwrap_or_else(|| BTBN_RELEASES_PAGE.into());
    let date_of = |a: &GhAsset| a.updated_at.as_deref().or(rel.published_at.as_deref()).and_then(parse_iso_date);
    let fmt_date = |d: Option<(i32, u32, u32)>| d.map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}")).unwrap_or_default();
    match channel {
        UpdateChannel::Stable => {
            let best = rel
                .assets
                .iter()
                .filter_map(|a| parse_btbn_stable_name(&a.name).map(|v| (v, a)))
                .max_by_key(|(v, _)| *v)
                .ok_or("No win64 GPL release-branch build found in the BtbN release")?;
            let ((maj, min), a) = best;
            let date = date_of(a);
            let version = FfVersion { raw: format!("n{maj}.{min}"), semver: Some((maj, min, 0)), date, git: false };
            Ok(AvailableBuild {
                source: UpdateSource::BtbN,
                channel,
                label: format!("n{maj}.{min} (built {})", fmt_date(date)),
                version,
                url: a.browser_download_url.clone(),
                asset_name: a.name.clone(),
                size: a.size,
                sha256: sha_from_digest(&a.digest),
                archive: ArchiveKind::Zip,
                notes_url: format!("https://github.com/FFmpeg/FFmpeg/blob/release/{maj}.{min}/Changelog"),
                page_url,
            })
        }
        UpdateChannel::Latest => {
            let a = rel
                .assets
                .iter()
                .find(|a| a.name == BTBN_MASTER_ASSET)
                .ok_or("No win64 GPL master build found in the BtbN release")?;
            let date = date_of(a);
            let version = FfVersion { raw: format!("master {}", fmt_date(date)), semver: None, date, git: true };
            Ok(AvailableBuild {
                source: UpdateSource::BtbN,
                channel,
                label: format!("git master (built {})", fmt_date(date)),
                version,
                url: a.browser_download_url.clone(),
                asset_name: a.name.clone(),
                size: a.size,
                sha256: sha_from_digest(&a.digest),
                archive: ArchiveKind::Zip,
                notes_url: "https://github.com/FFmpeg/FFmpeg/commits/master".into(),
                page_url,
            })
        }
    }
}

/// Build description for gyan.dev from its version file (+ optional .sha256 text).
pub fn gyan_build(channel: UpdateChannel, version_text: &str, sha_text: Option<&str>) -> Result<AvailableBuild, String> {
    let ver = version_text.trim();
    if ver.is_empty() || ver.len() > 80 || ver.contains('<') || ver.contains(char::is_whitespace) {
        return Err("Unexpected gyan.dev version file contents".into());
    }
    let version = parse_version(ver);
    let (url, archive, asset_name, notes) = match channel {
        UpdateChannel::Stable => {
            if version.semver.is_none() {
                return Err(format!("Unexpected gyan.dev release version: {ver}"));
            }
            (GYAN_RELEASE_ZIP, ArchiveKind::Zip, format!("ffmpeg-{ver}-essentials_build.zip"), format!("https://github.com/FFmpeg/FFmpeg/blob/n{ver}/Changelog"))
        }
        UpdateChannel::Latest => (GYAN_GIT_7Z, ArchiveKind::SevenZ, format!("ffmpeg-{ver}-essentials_build.7z"), "https://github.com/FFmpeg/FFmpeg/commits/master".into()),
    };
    Ok(AvailableBuild {
        source: UpdateSource::Gyan,
        channel,
        label: ver.to_string(),
        version,
        url: url.into(),
        asset_name,
        size: None,
        sha256: sha_text.and_then(parse_sha256_text),
        archive,
        notes_url: notes,
        page_url: GYAN_PAGE.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed copy of the live API response captured on 2026-10-31.
    const BTBN: &str = r#"{
      "tag_name":"latest","name":"Latest Auto-Build (2026-10-31 12:59)","published_at":"2026-10-31T13:20:35Z",
      "html_url":"https://github.com/BtbN/FFmpeg-Builds/releases/tag/latest",
      "assets":[
        {"name":"ffmpeg-master-latest-win64-gpl-shared.zip","browser_download_url":"https://x/shared.zip","size":1,"digest":"sha256:c1ecec0fa8ea13b690fa79a10040e5d178843ea0fd68c6886e9be0979c922383"},
        {"name":"ffmpeg-master-latest-win64-gpl.zip","browser_download_url":"https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip","size":201661328,"digest":"sha256:d12357d3823203c176a5b94bfcdbd8f6d9a76de6d05101e766e851da2fee1a51","updated_at":"2026-10-31T13:10:00Z"},
        {"name":"ffmpeg-n8.1-latest-win64-gpl-8.1.zip","browser_download_url":"https://x/81.zip","size":194180572,"digest":"sha256:10f00905240b0409672c4fb1e9eb841f22aaf81b36f95827e57d483d9b144817"},
        {"name":"ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip","browser_download_url":"https://x/90s.zip","size":2,"digest":"sha256:247f743ea6089d67729696b2763a28c16fac61778c7c2b0ad5068d85e2631438"},
        {"name":"ffmpeg-n9.0-latest-win64-gpl-9.0.zip","browser_download_url":"https://x/90.zip","size":195108456,"digest":"sha256:3cfc8bfc41d2a01b6c98be8a37e212048e16dd3e71b869be48e214a74ee2bde1"},
        {"name":"ffmpeg-n9.0-latest-win64-lgpl-9.0.zip","browser_download_url":"https://x/90l.zip","size":3},
        {"name":"ffmpeg-n10.0-latest-linux64-gpl-10.0.tar.xz","browser_download_url":"https://x/linux.tar.xz","size":3}
      ]}"#;

    #[test]
    fn stable_name_parsing() {
        assert_eq!(parse_btbn_stable_name("ffmpeg-n9.0-latest-win64-gpl-9.0.zip"), Some((9, 0)));
        assert_eq!(parse_btbn_stable_name("ffmpeg-n10.12-latest-win64-gpl-10.12.zip"), Some((10, 12)));
        assert_eq!(parse_btbn_stable_name("ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip"), None);
        assert_eq!(parse_btbn_stable_name("ffmpeg-n9.0-latest-win64-lgpl-9.0.zip"), None);
        assert_eq!(parse_btbn_stable_name("ffmpeg-n9.0-latest-winarm64-gpl-9.0.zip"), None);
        assert_eq!(parse_btbn_stable_name("ffmpeg-master-latest-win64-gpl.zip"), None);
    }

    #[test]
    fn btbn_stable_picks_newest_branch() {
        let b = parse_btbn_release(BTBN, UpdateChannel::Stable).unwrap();
        assert_eq!(b.asset_name, "ffmpeg-n9.0-latest-win64-gpl-9.0.zip");
        assert_eq!(b.version.semver, Some((9, 0, 0)));
        assert_eq!(b.version.date, Some((2026, 10, 31)));
        assert_eq!(b.sha256.as_deref(), Some("3cfc8bfc41d2a01b6c98be8a37e212048e16dd3e71b869be48e214a74ee2bde1"));
        assert_eq!(b.archive, ArchiveKind::Zip);
    }

    #[test]
    fn btbn_master() {
        let b = parse_btbn_release(BTBN, UpdateChannel::Latest).unwrap();
        assert_eq!(b.asset_name, BTBN_MASTER_ASSET);
        assert!(b.version.git);
        assert_eq!(b.size, Some(201661328));
    }

    #[test]
    fn btbn_errors() {
        assert!(parse_btbn_release("{\"assets\":[]}", UpdateChannel::Stable).is_err());
        assert!(parse_btbn_release("<html>", UpdateChannel::Stable).is_err());
    }

    #[test]
    fn gyan() {
        let b = gyan_build(UpdateChannel::Stable, "9.0.2\n", Some("60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba")).unwrap();
        assert_eq!(b.version.semver, Some((9, 0, 2)));
        assert_eq!(b.url, GYAN_RELEASE_ZIP);
        assert_eq!(b.sha256.as_deref(), Some("60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba"));
        let g = gyan_build(UpdateChannel::Latest, "2026-10-29-git-31ba826726", None).unwrap();
        assert!(g.version.git);
        assert_eq!(g.archive, ArchiveKind::SevenZ);
        assert!(gyan_build(UpdateChannel::Stable, "<!DOCTYPE html>", None).is_err());
    }

    #[test]
    fn sha_text() {
        assert_eq!(parse_sha256_text("ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  file.zip").unwrap(),
                   "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");
        assert_eq!(parse_sha256_text("<html>"), None);
    }

    /// Live network check (run with `cargo test -- --ignored live_`).
    #[test]
    #[ignore]
    fn live_sources() {
        for src in super::super::UpdateSource::ALL {
            for ch in super::super::UpdateChannel::ALL {
                let b = super::super::check(src, ch).unwrap();
                println!("{src:?} {ch:?}: {} {} sha={:?}", b.label, b.url, b.sha256);
                assert!(b.sha256.is_some());
            }
        }
    }
}
