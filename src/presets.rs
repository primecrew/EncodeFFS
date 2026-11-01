//! Presets: named, saved configurations of an operation type ("kind").
//!
//! A preset stores the option values of its kind as opaque JSON (owned by the
//! kind's form). Built-in presets ship with `values = null`, meaning "the kind's
//! defaults derived from Settings", so resetting a built-in just clears its values.
//! Presets live in their own JSON file next to the settings file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{settings_path, PORTABLE_FILE};

pub const PORTABLE_PRESETS_FILE: &str = "FFmpegEncoder.presets.json";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    /// Operation type, e.g. "reencode", "custom".
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    /// Id of the factory preset this one can be reset to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    /// Option values (null = kind defaults).
    #[serde(default)]
    pub values: Value,
    /// Shown in the "Favorites" group at the top of the sidebar.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// Optional folder in the sidebar ("" = none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    pub ffmpeg_encoder_presets: u32,
    pub presets: Vec<Preset>,
    /// Factory ids this file has already seen; newer built-ins are appended on
    /// load, deleted ones stay deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_builtins: Vec<String>,
}

/// Built-ins that existed in 0.2.x (files without `known_builtins`).
const BUILTINS_V02: &[&str] = &[
    "builtin-reencode", "builtin-still", "builtin-commentary", "builtin-extract", "builtin-replace",
    "builtin-mix", "builtin-batch", "builtin-subs", "builtin-remux", "builtin-custom",
];

/// (factory id, kind, name, description)
pub const BUILTINS: &[(&str, &str, &str, &str)] = &[
    ("builtin-reencode", "reencode", "Re-encode video", "libx264 + AAC with trim, resize, chapters, passthrough, gain/compressor and deinterlacing."),
    ("builtin-still", "still", "MP4 from still image", "One image + one audio file -> H.264/AAC MP4."),
    ("builtin-commentary", "commentary", "Add commentary track", "Copy video and all audio, add an audio file as a new AAC track."),
    ("builtin-extract", "extract", "Extract audio to WAV", "One audio track -> 16-bit PCM WAV."),
    ("builtin-replace", "replace_audio", "Replace primary audio with WAV", "Copy video, replace the audio with an audio file encoded to AAC."),
    ("builtin-mix", "mix", "Mix audio tracks to one (MKV)", "Mix every audio track into one AAC track; video copied."),
    ("builtin-batch", "batch", "Batch re-encode folder", "Re-encode every video in a folder with shared settings."),
    ("builtin-subs", "subs", "Copy subtitle tracks", "Copy subtitle tracks from one file into another."),
    ("builtin-remux", "remux", "Remux MKV -> MP4", "Stream-copy every MKV in a folder to MP4 (never overwrites)."),
    ("builtin-join", "join", "Join / stitch files", "Concatenate several videos into one – lossless when they match, or re-encoded – with a chapter per file."),
    ("builtin-custom", "custom", "Custom pipeline", "Build your own: video / audio / subtitle actions, trim, chapters, container, extra ffmpeg args."),
    ("builtin-addchapters", "custom", "Add chapters (no re-encode)", "Copy every stream and add chapters (every 5 min, your own list, or imported) – no quality loss."),
];

/// Factory option values (null = the kind's defaults).
pub fn factory_values(id: &str) -> Value {
    match id {
        "builtin-addchapters" => serde_json::json!({
            "video": "Copy", "audio": "Copy", "subs": "Copy",
            "chapter_mode": "Auto", "chapter_interval_min": 5.0,
            "container": "same", "suffix": "_chapters", "faststart": true
        }),
        _ => Value::Null,
    }
}

pub fn factory(id: &str) -> Option<Preset> {
    BUILTINS.iter().find(|b| b.0 == id).map(|&(id, kind, name, desc)| Preset {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        description: desc.into(),
        builtin: Some(id.into()),
        values: factory_values(id),
        pinned: false,
        category: String::new(),
    })
}

pub fn factory_all() -> Vec<Preset> {
    BUILTINS.iter().filter_map(|b| factory(b.0)).collect()
}

/// Factory presets whose id is no longer present (deleted by the user).
pub fn missing_builtins(list: &[Preset]) -> Vec<Preset> {
    factory_all().into_iter().filter(|f| !list.iter().any(|p| p.builtin.as_deref() == Some(f.id.as_str()))).collect()
}

/// Where presets are stored: next to the settings file.
pub fn presets_path() -> PathBuf {
    presets_path_for(&settings_path())
}

pub fn presets_path_for(settings: &Path) -> PathBuf {
    let dir = settings.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    if settings.file_name().map(|n| n == PORTABLE_FILE).unwrap_or(false) {
        dir.join(PORTABLE_PRESETS_FILE)
    } else {
        dir.join("presets.json")
    }
}

pub fn new_id() -> String {
    format!("p{}", crate::util::unique_id())
}

/// "Name", "Name (2)", "Name (3)" … not used by any preset.
pub fn unique_name(list: &[Preset], base: &str) -> String {
    let base = base.trim();
    let base = if base.is_empty() { "Preset" } else { base };
    let taken = |n: &str| list.iter().any(|p| p.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).unwrap()
}

/// Parse a presets file: the full file format, a bare array, or one preset object.
pub fn parse(text: &str) -> Result<Vec<Preset>, String> {
    parse_file(text).map(|f| f.0)
}

/// Like [`parse`], plus the file's `known_builtins` (None for arrays / single presets).
pub fn parse_file(text: &str) -> Result<(Vec<Preset>, Option<Vec<String>>), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Not valid JSON: {e}"))?;
    let mut known = None;
    let list: Vec<Preset> = if v.get("presets").is_some() {
        let f = serde_json::from_value::<PresetFile>(v).map_err(|e| format!("Not a presets file: {e}"))?;
        known = Some(f.known_builtins);
        f.presets
    } else if v.is_array() {
        serde_json::from_value(v).map_err(|e| format!("Not a list of presets: {e}"))?
    } else {
        vec![serde_json::from_value(v).map_err(|e| format!("Not a preset: {e}"))?]
    };
    for p in &list {
        if p.kind.trim().is_empty() {
            return Err(format!("Preset \"{}\" has no type", p.name));
        }
    }
    Ok((list, known))
}

pub fn to_json(list: &[Preset]) -> String {
    serde_json::to_string_pretty(&PresetFile {
        ffmpeg_encoder_presets: FORMAT_VERSION,
        presets: list.to_vec(),
        known_builtins: BUILTINS.iter().map(|b| b.0.to_string()).collect(),
    })
    .unwrap_or_default()
}

/// Append factory presets that are newer than the file (not deleted ones).
fn add_new_builtins(list: &mut Vec<Preset>, known: &[String]) {
    let known: Vec<&str> = if known.is_empty() { BUILTINS_V02.to_vec() } else { known.iter().map(|s| s.as_str()).collect() };
    for f in factory_all() {
        if !known.contains(&f.id.as_str()) && !list.iter().any(|p| p.builtin.as_deref() == Some(f.id.as_str())) {
            let mut f = f;
            f.name = unique_name(list, &f.name);
            list.push(f);
        }
    }
}

/// Load presets; a missing file gives the factory list. A corrupt file is
/// renamed to `*.bad` (so it is never silently overwritten) and the factory list is used.
pub fn load_from(path: &Path) -> (Vec<Preset>, Option<String>) {
    match std::fs::read_to_string(path) {
        Err(_) => (factory_all(), None),
        Ok(t) => match parse_file(&t) {
            Ok((mut l, known)) => {
                add_new_builtins(&mut l, &known.unwrap_or_default());
                (l, None)
            }
            Err(e) => {
                let bad = path.with_extension(format!("bad-{}", crate::util::now_unix()));
                let _ = std::fs::rename(path, &bad);
                (factory_all(), Some(format!("Presets file was unreadable ({e}); moved to {} and started from the built-ins.", bad.display())))
            }
        },
    }
}

pub fn save_to(path: &Path, list: &[Preset]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_json(list)).map_err(|e| format!("Could not save presets to {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Could not save presets to {}: {e}", path.display()))
}

/// Prepare imported presets for insertion: fresh ids, unique names, unknown kinds rejected.
/// A built-in link is kept only when that built-in is missing from `existing`.
pub fn prepare_import(existing: &[Preset], imported: Vec<Preset>, known_kinds: &[&str]) -> (Vec<Preset>, Vec<String>) {
    let mut out: Vec<Preset> = vec![];
    let mut skipped = vec![];
    for mut p in imported {
        if !known_kinds.contains(&p.kind.as_str()) {
            skipped.push(format!("{} (unknown type \"{}\")", p.name, p.kind));
            continue;
        }
        let all: Vec<Preset> = existing.iter().chain(out.iter()).cloned().collect();
        if let Some(b) = &p.builtin {
            if all.iter().any(|x| x.builtin.as_deref() == Some(b.as_str())) || factory(b).is_none() {
                p.builtin = None;
            }
        }
        p.id = new_id();
        p.name = unique_name(&all, &p.name);
        out.push(p);
    }
    (out, skipped)
}

/// Remove per-run keys (input files, typed output names) from a form's JSON.
pub fn strip_run_fields(mut v: Value, fields: &[&str]) -> Value {
    if let Value::Object(m) = &mut v {
        for f in fields {
            m.remove(*f);
        }
    }
    v
}

/// `base` with every key of `overlay` replaced (shallow).
pub fn overlay(mut base: Value, overlay: &Value) -> Value {
    if let (Value::Object(b), Value::Object(o)) = (&mut base, overlay) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
        return base;
    }
    overlay.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn factory_list() {
        let f = factory_all();
        assert_eq!(f.len(), BUILTINS.len());
        assert!(f.iter().all(|p| p.builtin.as_deref() == Some(p.id.as_str())));
        assert!(f.iter().filter(|p| p.id != "builtin-addchapters").all(|p| p.values.is_null()));
        assert_eq!(factory("builtin-addchapters").unwrap().values["container"], "same");
        let ids: std::collections::HashSet<_> = f.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids.len(), f.len());
    }

    #[test]
    fn missing_after_delete() {
        let mut l = factory_all();
        l.retain(|p| p.kind != "mix");
        let m = missing_builtins(&l);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].kind, "mix");
    }

    #[test]
    fn names() {
        let mut l = factory_all();
        assert_eq!(unique_name(&l, "Fresh"), "Fresh");
        assert_eq!(unique_name(&l, "re-encode video"), "re-encode video (2)");
        l.push(Preset { name: "Re-encode video (2)".into(), ..factory("builtin-reencode").unwrap() });
        assert_eq!(unique_name(&l, "Re-encode video"), "Re-encode video (3)");
        assert_eq!(unique_name(&[], "  "), "Preset");
    }

    #[test]
    fn roundtrip_file_and_parse_variants() {
        let dir = std::env::temp_dir().join(format!("pre_{}", crate::util::unique_id()));
        let path = dir.join("p.json");
        let mut l = factory_all();
        l[0].values = json!({"video": {"crf": 18}});
        save_to(&path, &l).unwrap();
        assert_eq!(load_from(&path), (l.clone(), None));
        // bare array and single object
        let one = serde_json::to_string(&l[1]).unwrap();
        assert_eq!(parse(&one).unwrap(), vec![l[1].clone()]);
        let arr = serde_json::to_string(&l[..2]).unwrap();
        assert_eq!(parse(&arr).unwrap().len(), 2);
        assert!(parse("{\"name\":\"x\"}").is_err());
        assert!(parse("nope").is_err());
        // corrupt file is moved aside, not overwritten
        std::fs::write(&path, "{broken").unwrap();
        let (l2, msg) = load_from(&path);
        assert_eq!(l2, factory_all());
        assert!(msg.unwrap().contains("unreadable"));
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        // missing file -> factory
        assert_eq!(load_from(&dir.join("none.json")).0, factory_all());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn import_rules() {
        let existing = factory_all();
        let mut a = factory("builtin-reencode").unwrap();
        a.values = json!({"x": 1});
        let b = Preset { id: "zzz".into(), name: "Mine".into(), kind: "custom".into(), description: String::new(), builtin: None, values: Value::Null, pinned: false, category: String::new() };
        let c = Preset { kind: "teleport".into(), ..b.clone() };
        let (out, skipped) = prepare_import(&existing, vec![a, b.clone(), c, b], &["reencode", "custom"]);
        assert_eq!(skipped.len(), 1);
        assert_eq!(out.len(), 3);
        assert!(out[0].builtin.is_none(), "built-in already present -> plain copy");
        assert_eq!(out[0].name, "Re-encode video (2)");
        assert_eq!(out[1].name, "Mine");
        assert_eq!(out[2].name, "Mine (2)");
        assert!(out.iter().all(|p| p.id.starts_with('p') && p.id != "zzz"));
        // re-importing a deleted built-in restores the link
        let mut ex2 = factory_all();
        ex2.retain(|p| p.kind != "reencode");
        let (out, _) = prepare_import(&ex2, vec![factory("builtin-reencode").unwrap()], &["reencode"]);
        assert_eq!(out[0].builtin.as_deref(), Some("builtin-reencode"));
    }

    #[test]
    fn strip_and_overlay() {
        let full = json!({"input": "a.mkv", "out_name": "x", "crf": 18});
        let s = strip_run_fields(full.clone(), &["input", "out_name"]);
        assert_eq!(s, json!({"crf": 18}));
        let reverted = overlay(json!({"input": "a.mkv", "out_name": "x", "crf": 30}), &s);
        assert_eq!(reverted, full);
        assert_eq!(overlay(Value::Null, &s), s);
    }

    #[test]
    fn new_builtins_appended_deleted_stay_deleted() {
        let dir = std::env::temp_dir().join(format!("pre_{}", crate::util::unique_id()));
        let path = dir.join("p.json");
        std::fs::create_dir_all(&dir).unwrap();
        // a 0.2.x file (no known_builtins) where the user deleted "mix"
        let mut old: Vec<Preset> = factory_all().into_iter().filter(|p| p.id != "builtin-addchapters" && p.id != "builtin-mix").collect();
        old[0].pinned = true;
        old[0].category = "Daily".into();
        let v02 = serde_json::json!({"ffmpeg_encoder_presets": 1, "presets": old});
        std::fs::write(&path, v02.to_string()).unwrap();
        let (l, _) = load_from(&path);
        assert!(l.iter().any(|p| p.id == "builtin-addchapters"), "new built-in appended");
        assert!(!l.iter().any(|p| p.id == "builtin-mix"), "deleted built-in not resurrected");
        assert!(l[0].pinned && l[0].category == "Daily");
        // after saving, a deleted new built-in stays deleted
        let l2: Vec<Preset> = l.into_iter().filter(|p| p.id != "builtin-addchapters").collect();
        save_to(&path, &l2).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("known_builtins"));
        assert!(!text.contains("\"pinned\": false"), "defaults not written");
        assert_eq!(load_from(&path).0, l2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths() {
        assert_eq!(presets_path_for(Path::new("/x/FFmpegEncoder.settings.json")), PathBuf::from("/x/FFmpegEncoder.presets.json"));
        assert_eq!(presets_path_for(Path::new("/r/FFmpegEncoder/settings.json")), PathBuf::from("/r/FFmpegEncoder/presets.json"));
    }
}
