//! Recent folders for folder-based presets (Remux, Batch re-encode, Custom in
//! folder mode). Suggest only; Settings default input/output folders stay as set.

/// Default number of recent folders to keep.
pub const DEFAULT_LIMIT: u32 = 4;
/// Inclusive maximum for the Settings option.
pub const MAX_LIMIT: u32 = 20;

/// Clamp a stored limit into 0..=MAX_LIMIT.
pub fn clamp_limit(n: u32) -> u32 {
    n.min(MAX_LIMIT)
}

/// Comparison key: trim, drop a trailing slash (except a drive root), and
/// lowercase. The app is Windows-first, so folder identity is case-insensitive.
pub fn folder_key(path: &str) -> String {
    normalize(path).to_ascii_lowercase()
}

fn normalize(path: &str) -> String {
    let s = path.trim();
    if s.is_empty() {
        return String::new();
    }
    let bytes = s.as_bytes();
    // Keep "C:\" / "C:/" / "/" as-is (aside from trim).
    let is_drive_root = bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if is_drive_root || s == "/" {
        return s.to_string();
    }
    s.trim_end_matches(['\\', '/']).to_string()
}

/// Insert `path` at the front of `list` (newest first), drop earlier copies
/// (case-insensitive), and truncate to `limit`. A limit of 0 clears the list
/// and stores nothing. Empty or whitespace-only paths are ignored.
pub fn remember(list: &mut Vec<String>, path: &str, limit: u32) {
    let limit = clamp_limit(limit);
    if limit == 0 {
        list.clear();
        return;
    }
    let stored = path.trim();
    if stored.is_empty() {
        return;
    }
    let key = folder_key(stored);
    if key.is_empty() {
        return;
    }
    list.retain(|p| folder_key(p) != key);
    list.insert(0, stored.to_string());
    list.truncate(limit as usize);
}

/// Entries for a folder field's drop-down: the Settings default (when set) first,
/// then recents newest first, de-duplicated against the default. `exists` is
/// used to skip missing recents (cheap `is_dir` in the UI). The default is
/// included when it is non-empty, even if `exists` is false, so a typed default
/// stays visible.
pub fn menu_entries(default_folder: &str, recents: &[String], exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let def = default_folder.trim();
    let def_key = if def.is_empty() { String::new() } else { folder_key(def) };
    if !def.is_empty() {
        out.push(def.to_string());
    }
    for r in recents {
        let t = r.trim();
        if t.is_empty() {
            continue;
        }
        if !def_key.is_empty() && folder_key(t) == def_key {
            continue;
        }
        if !exists(t) {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// Folder the Browse dialog should open on: the field's current directory if it
/// exists, otherwise the newest recent that exists, otherwise the Settings
/// default if it exists.
pub fn browse_dir(current: &str, default_folder: &str, recents: &[String], exists: impl Fn(&str) -> bool) -> Option<String> {
    let cur = current.trim();
    if !cur.is_empty() && exists(cur) {
        return Some(cur.to_string());
    }
    // If the field holds a file path, try its parent (same as the old start_dir helper).
    if !cur.is_empty() {
        let p = std::path::Path::new(cur);
        if let Some(parent) = p.parent().and_then(|d| d.to_str()) {
            if !parent.is_empty() && exists(parent) {
                return Some(parent.to_string());
            }
        }
    }
    for r in recents {
        let t = r.trim();
        if !t.is_empty() && exists(t) {
            return Some(t.to_string());
        }
    }
    let def = default_folder.trim();
    if !def.is_empty() && exists(def) {
        return Some(def.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_newest_first_dedupes_and_limits() {
        let mut v = Vec::new();
        remember(&mut v, r"C:\Videos\One", 4);
        remember(&mut v, r"C:\Videos\Two", 4);
        remember(&mut v, r"C:\Videos\Three", 4);
        assert_eq!(v, [r"C:\Videos\Three", r"C:\Videos\Two", r"C:\Videos\One"]);
        // case-insensitive dedupe of a Windows path; newest spelling is kept
        remember(&mut v, r"c:\videos\one", 4);
        assert_eq!(v, [r"c:\videos\one", r"C:\Videos\Three", r"C:\Videos\Two"]);
        remember(&mut v, r"C:\Videos\Four", 4);
        remember(&mut v, r"C:\Videos\Five", 4);
        assert_eq!(v.len(), 4);
        assert_eq!(v[0], r"C:\Videos\Five");
        assert!(!v.iter().any(|p| folder_key(p) == folder_key(r"C:\Videos\Two")), "{v:?}");
    }

    #[test]
    fn limit_zero_stores_nothing() {
        let mut v = vec![r"C:\A".into(), r"C:\B".into()];
        remember(&mut v, r"C:\C", 0);
        assert!(v.is_empty());
    }

    #[test]
    fn trailing_slash_and_blank_are_ignored_or_normalized() {
        let mut v = Vec::new();
        remember(&mut v, r"C:\Videos\Out\", 4);
        remember(&mut v, r"C:\Videos\Out", 4);
        assert_eq!(v, [r"C:\Videos\Out"]);
        remember(&mut v, "   ", 4);
        assert_eq!(v, [r"C:\Videos\Out"]);
        remember(&mut v, r"C:\", 4);
        assert_eq!(v[0], r"C:\");
    }

    #[test]
    fn menu_puts_default_first_then_recents_without_dups_or_missing() {
        let recents = vec![
            r"C:\Recent\New".into(),
            r"C:\Apps\Default".into(), // same as default, different case
            r"C:\Missing".into(),
            r"C:\Recent\Old".into(),
        ];
        let exists = |p: &str| folder_key(p) != folder_key(r"C:\Missing");
        let m = menu_entries(r"C:\Apps\Default", &recents, exists);
        assert_eq!(m, [r"C:\Apps\Default", r"C:\Recent\New", r"C:\Recent\Old"]);
        assert!(menu_entries("", &[], exists).is_empty());
        // default is listed even if exists() is false
        let m = menu_entries(r"C:\Apps\Default", &[], |_| false);
        assert_eq!(m, [r"C:\Apps\Default"]);
    }

    #[test]
    fn browse_prefers_current_then_recent_then_default() {
        let exists = |p: &str| matches!(p, r"C:\Last" | r"C:\Recent" | r"C:\Default");
        assert_eq!(browse_dir(r"C:\Last", r"C:\Default", &[r"C:\Recent".into()], exists).as_deref(), Some(r"C:\Last"));
        assert_eq!(browse_dir("", r"C:\Default", &[r"C:\Recent".into()], exists).as_deref(), Some(r"C:\Recent"));
        assert_eq!(browse_dir("", r"C:\Default", &[r"C:\Gone".into()], exists).as_deref(), Some(r"C:\Default"));
        assert_eq!(browse_dir("", "", &[r"C:\Gone".into()], exists), None);
    }

    #[test]
    fn clamp_limit_caps_at_max() {
        assert_eq!(clamp_limit(0), 0);
        assert_eq!(clamp_limit(4), 4);
        assert_eq!(clamp_limit(20), 20);
        assert_eq!(clamp_limit(99), 20);
    }
}
