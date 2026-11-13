// Windows VERSIONINFO mapping for the crate version (used by `build.rs` and tests).
//
// The numeric FILEVERSION/PRODUCTVERSION quad is the three core version numbers plus,
// as the fourth component, the last number in a pre-release suffix (or 0).
// Examples: `0.6.0` → 0.6.0.0, `0.6.0-rc.3` → 0.6.0.3, `0.5.5-codecs.3` → 0.5.5.3.

/// Parse `CARGO_PKG_VERSION` into a Windows file-version quad.
pub fn file_version_quad(pkg: &str) -> [u16; 4] {
    let (core, pre) = match pkg.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (pkg, None),
    };
    let mut parts = core.split('.');
    let major = parse_u16(parts.next());
    let minor = parse_u16(parts.next());
    let patch = parse_u16(parts.next());
    let build = pre
        .and_then(|p| p.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).next_back())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    [major, minor, patch, build]
}

fn parse_u16(s: Option<&str>) -> u16 {
    s.and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Dotted string for the VERSIONINFO `FileVersion` / `ProductVersion` values.
pub fn file_version_string(pkg: &str) -> String {
    let [a, b, c, d] = file_version_quad(pkg);
    format!("{a}.{b}.{c}.{d}")
}

/// `FILEVERSION a,b,c,d` comma form.
pub fn file_version_commas(pkg: &str) -> String {
    let [a, b, c, d] = file_version_quad(pkg);
    format!("{a},{b},{c},{d}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_versions_map_to_a_windows_quad() {
        assert_eq!(file_version_quad("0.6.0"), [0, 6, 0, 0]);
        assert_eq!(file_version_string("0.6.0"), "0.6.0.0");
        assert_eq!(file_version_commas("0.6.0-rc.3"), "0,6,0,3");
        assert_eq!(file_version_quad("0.6.0-rc.3"), [0, 6, 0, 3]);
        assert_eq!(file_version_quad("0.5.5-codecs.3"), [0, 5, 5, 3]);
        assert_eq!(file_version_quad("0.6.0-dev.1"), [0, 6, 0, 1]);
        assert_eq!(file_version_string("0.5.1"), "0.5.1.0");
    }
}
