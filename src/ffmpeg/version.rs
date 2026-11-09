//! `ffmpeg -version` / `-encoders` parsing and version comparison.

use std::path::Path;

/// A parsed FFmpeg version string, e.g.
/// * BtbN release branch: `n8.1-15-gabcdef0123-20261030`
/// * BtbN master: `N-121234-gabcdef0123-20261030`
/// * gyan release: `9.0.2-essentials_build-www.gyan.dev`
/// * gyan git: `2026-10-29-git-31ba826726-essentials_build-www.gyan.dev`
/// * distro: `7.1.5-0+deb13u1`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FfVersion {
    pub raw: String,
    pub semver: Option<(u32, u32, u32)>,
    /// Build/commit date (y, m, d) when the string carries one.
    pub date: Option<(i32, u32, u32)>,
    /// Development (git master) build.
    pub git: bool,
}

fn valid_date(y: i32, m: u32, d: u32) -> bool {
    (2000..2200).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d)
}

/// Days since 2000-01-01 (approximate is fine for comparisons).
pub fn day_number((y, m, d): (i32, u32, u32)) -> i64 {
    // Howard Hinnant's days_from_civil
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Parse "2026-10-31" or "2026-10-31T13:20:35Z".
pub fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> {
    let s = s.get(0..10)?;
    let y = s.get(0..4)?.parse().ok()?;
    let m = s.get(5..7)?.parse().ok()?;
    let d = s.get(8..10)?.parse().ok()?;
    (s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && valid_date(y, m, d)).then_some((y, m, d))
}

pub fn parse_version(raw: &str) -> FfVersion {
    let raw = raw.trim().to_string();
    let mut v = FfVersion { raw: raw.clone(), ..Default::default() };
    // gyan git: leading YYYY-MM-DD-git
    if let Some(d) = parse_iso_date(&raw) {
        v.date = Some(d);
        v.git = true;
        return v;
    }
    if raw.starts_with("N-") || raw.starts_with("git-") || raw.contains("-git-") {
        v.git = true;
    }
    // semver from leading "n8.1.2" / "8.1"
    let body = raw.strip_prefix('n').unwrap_or(&raw);
    if !v.git {
        let ver: String = body.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let nums: Vec<u32> = ver.split('.').filter_map(|x| x.parse().ok()).collect();
        if !nums.is_empty() {
            v.semver = Some((nums[0], *nums.get(1).unwrap_or(&0), *nums.get(2).unwrap_or(&0)));
        }
    }
    // trailing YYYYMMDD token (BtbN)
    for tok in raw.split(['-', '_', ' ']) {
        if tok.len() == 8 && tok.chars().all(|c| c.is_ascii_digit()) {
            let y = tok[0..4].parse().unwrap_or(0);
            let m = tok[4..6].parse().unwrap_or(0);
            let d = tok[6..8].parse().unwrap_or(0);
            if valid_date(y, m, d) {
                v.date = Some((y, m, d));
            }
        }
    }
    v
}

/// Extract "8.1-xyz" from the first line "ffmpeg version 8.1-xyz Copyright (c) ...".
pub fn version_from_output(out: &str) -> Option<String> {
    let line = out.lines().next()?;
    let rest = line.strip_prefix("ffmpeg version ")?;
    Some(rest.split(" Copyright").next().unwrap_or(rest).trim().to_string())
}

/// The "configuration:" line (build info for About).
pub fn configuration_from_output(out: &str) -> Option<String> {
    out.lines().find_map(|l| l.trim().strip_prefix("configuration:").map(|s| s.trim().to_string()))
}

/// Whether `ffmpeg -encoders` output lists an encoder with exactly this name.
pub fn has_encoder(encoders_out: &str, name: &str) -> bool {
    encoders_out.lines().any(|l| {
        let mut it = l.split_whitespace();
        let flags = it.next().unwrap_or("");
        flags.len() == 6 && it.next() == Some(name)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    UpdateAvailable,
    UpToDate,
    InstalledNewer,
    /// Can't compare (e.g. git build installed, release offered).
    Unknown,
}

/// Compare installed vs available. Release numbers win; when equal (or both
/// are git builds) compare build dates with a 1-day tolerance because the
/// build date and the publish date of the same build can differ by a day.
pub fn compare(installed: &FfVersion, available: &FfVersion) -> Verdict {
    let by_date = |a: Option<(i32, u32, u32)>, b: Option<(i32, u32, u32)>| match (a, b) {
        (Some(i), Some(av)) => {
            let diff = day_number(av) - day_number(i);
            if diff > 1 {
                Verdict::UpdateAvailable
            } else if diff < -1 {
                Verdict::InstalledNewer
            } else {
                Verdict::UpToDate
            }
        }
        _ => Verdict::Unknown,
    };
    match (installed.semver, available.semver) {
        // Same release branch (e.g. BtbN "n9.0" vs installed "n9.0.2-39-g…-20261031"):
        // branch builds track point releases, so the build date decides.
        (Some(i), Some(a)) if i != a && (i.0, i.1) == (a.0, a.1) && installed.date.is_some() && available.date.is_some() => {
            by_date(installed.date, available.date)
        }
        (Some(i), Some(a)) if i != a => {
            if a > i {
                Verdict::UpdateAvailable
            } else {
                Verdict::InstalledNewer
            }
        }
        (Some(_), Some(_)) => match by_date(installed.date, available.date) {
            Verdict::Unknown => Verdict::UpToDate,
            v => v,
        },
        _ if installed.git && available.git => by_date(installed.date, available.date),
        (Some(_), None) if available.git => by_date(installed.date, available.date),
        _ => Verdict::Unknown,
    }
}

/// Result of checking an ffmpeg binary.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FfmpegCheck {
    pub version: String,
    pub configuration: String,
    pub has_libx264: bool,
    pub has_aac: bool,
    /// True when `ffmpeg -filters` lists the xfade filter (needed for crossfades).
    pub has_xfade: bool,
}

/// Run `ffmpeg -version` and `ffmpeg -encoders`.
pub fn check_ffmpeg(ffmpeg: &Path) -> Result<FfmpegCheck, String> {
    let run = |args: &[&str]| -> Result<String, String> {
        let out = super::hidden_command(ffmpeg)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("Could not run {}: {e}", ffmpeg.display()))?;
        if !out.status.success() {
            return Err(format!("{} {} failed: {}", ffmpeg.display(), args.join(" "), String::from_utf8_lossy(&out.stderr)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let ver = run(&["-hide_banner", "-version"]).or_else(|_| run(&["-version"]))?;
    let ver_full = run(&["-version"]).unwrap_or_else(|_| ver.clone());
    let enc = run(&["-hide_banner", "-encoders"])?;
    let filters = run(&["-hide_banner", "-filters"]).unwrap_or_default();
    Ok(FfmpegCheck {
        version: version_from_output(&ver_full).unwrap_or_else(|| "unknown".into()),
        configuration: configuration_from_output(&ver_full).unwrap_or_default(),
        has_libx264: has_encoder(&enc, "libx264"),
        has_aac: has_encoder(&enc, "aac"),
        has_xfade: has_filter(&filters, "xfade"),
    })
}

/// True when `ffmpeg -filters` lists `name` as a filter (word boundary).
pub fn has_filter(filters_out: &str, name: &str) -> bool {
    filters_out.lines().any(|l| {
        let l = l.trim();
        // " .S. xfade             VV->V      Cross fade…"
        l.split_whitespace().nth(1) == Some(name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variants() {
        let v = parse_version("n8.1-15-gabcdef0123-20261030");
        assert_eq!(v.semver, Some((8, 1, 0)));
        assert_eq!(v.date, Some((2026, 10, 30)));
        assert!(!v.git);
        let v = parse_version("N-121234-gabcdef0123-20261030");
        assert!(v.git);
        assert_eq!(v.semver, None);
        assert_eq!(v.date, Some((2026, 10, 30)));
        let v = parse_version("9.0.2-essentials_build-www.gyan.dev");
        assert_eq!(v.semver, Some((9, 0, 2)));
        let v = parse_version("2026-10-29-git-31ba826726-essentials_build-www.gyan.dev");
        assert!(v.git);
        assert_eq!(v.date, Some((2026, 10, 29)));
        let v = parse_version("7.1.5-0+deb13u1");
        assert_eq!(v.semver, Some((7, 1, 5)));
    }

    #[test]
    fn has_filter_parses() {
        let f = "Filters:\n .S. xfade             VV->V      Cross fade\n ... scale             V->V       Scale\n";
        assert!(has_filter(f, "xfade"));
        assert!(!has_filter(f, "xfade_opencl"));
        assert!(!has_filter("", "xfade"));
    }

    #[test]
    fn output_parsing() {
        let out = "ffmpeg version 7.1.5-0+deb13u1 Copyright (c) 2000-2026 the FFmpeg developers\nbuilt with gcc 14\nconfiguration: --enable-gpl --enable-libx264\n";
        assert_eq!(version_from_output(out).unwrap(), "7.1.5-0+deb13u1");
        assert_eq!(configuration_from_output(out).unwrap(), "--enable-gpl --enable-libx264");
        let enc = "Encoders:\n V..... = Video\n ------\n V....D libx264              libx264 H.264\n A....D aac                  AAC (Advanced Audio Coding)\n A....D libfdk_aac   x\n";
        assert!(has_encoder(enc, "libx264"));
        assert!(has_encoder(enc, "aac"));
        assert!(!has_encoder(enc, "libx265"));
        assert!(!has_encoder(" A....D libfdk_aac  Fraunhofer", "aac"));
    }

    #[test]
    fn comparisons() {
        let inst = parse_version("n8.1-15-gabc-20261030");
        let mut avail = parse_version("n9.0");
        avail.date = Some((2026, 10, 31));
        assert_eq!(compare(&inst, &avail), Verdict::UpdateAvailable);
        let mut same = parse_version("n8.1");
        same.date = Some((2026, 10, 31));
        assert_eq!(compare(&inst, &same), Verdict::UpToDate);
        same.date = Some((2026, 11, 20));
        assert_eq!(compare(&inst, &same), Verdict::UpdateAvailable);
        assert_eq!(compare(&parse_version("9.0.2-full"), &parse_version("9.0.1")), Verdict::InstalledNewer);
        assert_eq!(compare(&parse_version("9.0.1"), &parse_version("9.0.2")), Verdict::UpdateAvailable);
        // git vs git by date
        let a = parse_version("N-1-gx-20261001");
        let b = parse_version("2026-10-29-git-31ba826726-essentials_build-www.gyan.dev");
        assert_eq!(compare(&a, &b), Verdict::UpdateAvailable);
        assert_eq!(compare(&b, &a), Verdict::InstalledNewer);
        // git installed, release offered
        assert_eq!(compare(&a, &parse_version("9.0.2")), Verdict::Unknown);
    }

    #[test]
    fn days() {
        assert_eq!(day_number((2026, 11, 1)) - day_number((2026, 10, 31)), 1);
        assert_eq!(parse_iso_date("2026-10-31T13:20:35Z"), Some((2026, 10, 31)));
        assert_eq!(parse_iso_date("9.0.2"), None);
    }

    #[test]
    fn same_branch_compares_by_date() {
        let inst = parse_version("n9.0.2-39-g7adfbe40dc-20261031");
        let mut avail = parse_version("n9.0");
        avail.date = Some((2026, 10, 31));
        assert_eq!(compare(&inst, &avail), Verdict::UpToDate);
        avail.date = Some((2026, 11, 20));
        assert_eq!(compare(&inst, &avail), Verdict::UpdateAvailable);
    }
}
