//! Presets: named, saved configurations of an operation type ("kind").
//!
//! A preset stores the option values of its kind as opaque JSON (owned by the
//! kind's form). Built-in presets ship with `values = null`, meaning "the kind's
//! defaults derived from Settings", so resetting a built-in just clears its values.
//! Presets live in their own JSON file next to the settings file.
//!
//! Built-in presets are read-only (0.5): their name, type, description and values
//! always come from the app code ([`BUILTINS`] / [`factory_values`]), so an app
//! update refreshes them. The presets file only remembers their order, favorite
//! pin, folder and whether they are hidden. Edits must be saved as a new preset;
//! built-ins edited by older versions are moved into a user preset on load
//! ([`enforce_builtins`]) so nothing is lost.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{settings_path, PORTABLE_FILE};

pub const PORTABLE_PRESETS_FILE: &str = "FFmpegEncoder-Codecs.presets.json";
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
    /// Hidden from the sidebar (built-ins can only be hidden, not deleted).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

impl Preset {
    /// A shipped, read-only preset.
    pub fn is_builtin(&self) -> bool {
        self.builtin.as_deref().map(|b| factory(b).is_some()).unwrap_or(false)
    }
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
    ("builtin-tracks", "tracks", "Audio & subtitles", "See a video's audio and subtitle tracks; keep, remove, rename, reorder; add commentary, another language or subtitle files; replace or extract audio – video is never re-encoded."),
    ("builtin-still", "still", "MP4 from still image", "One image + one audio file -> H.264/AAC MP4."),
    ("builtin-commentary", "commentary", "Add commentary track", "Copy video and all audio, add an audio file as a new AAC track."),
    ("builtin-extract", "extract", "Extract audio to WAV", "One audio track -> 16-bit PCM WAV."),
    ("builtin-replace", "replace_audio", "Replace primary audio with WAV", "Copy video, replace the audio with an audio file encoded to AAC."),
    ("builtin-mix", "mix", "Mix audio tracks to one (MKV)", "Mix every audio track into one AAC track; video copied."),
    ("builtin-batch", "batch", "Batch re-encode folder", "Re-encode every video in a folder with shared settings."),
    ("builtin-subs", "subs", "Copy subtitle tracks", "Copy subtitle tracks from one file into another."),
    ("builtin-remux", "remux", "Remux MKV -> MP4", "Stream-copy every MKV in a folder to MP4 (never overwrites)."),
    ("builtin-join", "join", "Join / stitch files", "Concatenate several videos into one – lossless when they match, or re-encoded – with a chapter per file."),
    ("builtin-frameserve", "frameserve", "Frameserver (AviSynth script)", "Encode straight from your editor through DebugMode FrameServer: pick the signpost .avi, the app writes the AviSynth script (ConvertToYUV420 by default) and encodes it with H.264 + AAC by default (or HEVC / AV1 and AC3 / copy)."),
    ("builtin-custom", "custom", "Custom pipeline", "Build your own: video / audio / subtitle actions, trim, chapters, container, extra ffmpeg args."),
    ("builtin-addchapters", "custom", "Add chapters (no re-encode)", "Copy every stream and add chapters (every 5 min, your own list, or imported) – no quality loss."),
    ("builtin-hevc-game", "reencode", "Archive – HEVC 10-bit (game footage)", "Experimental: libx265 CRF 20, preset slow, 10-bit, AAC. Smaller files than H.264; not every device/browser plays HEVC."),
    ("builtin-hevc-camera", "reencode", "Archive – HEVC 10-bit (camera footage)", "Experimental: libx265 CRF 18, preset slow, 10-bit, AAC. Smaller files than H.264; not every device/browser plays HEVC."),
    ("builtin-av1-game", "reencode", "Archive – AV1 10-bit (game footage)", "Experimental: SVT-AV1 CRF 28, preset 5, 10-bit, AAC. Very small files; needs a recent player/GPU to decode."),
    ("builtin-av1-camera", "reencode", "Archive – AV1 10-bit (camera footage)", "Experimental: SVT-AV1 CRF 26, preset 5, 10-bit, film grain off, AAC. Very small files; needs a recent player/GPU to decode."),
];

/// Sidebar folder the experimental-codec built-ins start in (below the others).
pub const EXPERIMENTAL_GROUP: &str = "Experimental codecs";

/// Factory option values (null = the kind's defaults).
pub fn factory_values(id: &str) -> Value {
    match id {
        "builtin-addchapters" => serde_json::json!({
            "video": "Copy", "audio": "Copy", "subs": "Copy",
            "chapter_mode": "Auto", "chapter_interval_min": 5.0,
            "container": "same", "suffix": "_chapters", "faststart": true
        }),
        "builtin-hevc-game" | "builtin-hevc-camera" => serde_json::json!({
            "video": { "codec": "Hevc", "crf": if id.ends_with("game") { 20 } else { 18 }, "preset": "slow", "tune": null, "ten_bit": true },
            "suffix": "_hevc"
        }),
        "builtin-av1-game" | "builtin-av1-camera" => serde_json::json!({
            "video": { "codec": "Av1", "crf": if id.ends_with("game") { 28 } else { 26 }, "svt_preset": 5, "ten_bit": true, "film_grain": 0 },
            "suffix": "_av1"
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
        category: if desc.starts_with("Experimental:") { EXPERIMENTAL_GROUP.into() } else { String::new() },
        hidden: false,
    })
}

pub fn factory_all() -> Vec<Preset> {
    BUILTINS.iter().filter_map(|b| factory(b.0)).collect()
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
    // built-ins: values come from the app code, only order / pin / folder / hidden are stored
    let presets = list.iter().map(|p| if p.is_builtin() { Preset { values: Value::Null, ..p.clone() } } else { p.clone() }).collect();
    serde_json::to_string_pretty(&PresetFile {
        ffmpeg_encoder_presets: FORMAT_VERSION,
        presets,
        known_builtins: BUILTINS.iter().map(|b| b.0.to_string()).collect(),
    })
    .unwrap_or_default()
}

/// Default test for "the stored built-in differs from the factory one" (raw JSON).
/// The app passes a smarter test that compares the resolved form values.
pub fn differs_raw(p: &Preset) -> bool {
    let Some(f) = p.builtin.as_deref().and_then(factory) else { return false };
    p.name != f.name || p.kind != f.kind || (!p.values.is_null() && p.values != f.values)
}

/// Make the built-ins read-only copies of the factory definitions:
/// - a stored built-in that differs (`differs`) keeps the user's version as a new
///   user preset right after it ("<name> (your edits)", or the name they gave it);
/// - every factory preset is present exactly once (missing ones are added: new
///   ones visible, ones deleted in older versions hidden);
/// - duplicate or unknown built-in links become user presets; user presets never
///   use a factory id or a built-in's name.
/// Returns plain-language notes for the log.
pub fn enforce_builtins(list: &mut Vec<Preset>, known: &[String], differs: &dyn Fn(&Preset) -> bool) -> Vec<String> {
    let mut notes = vec![];
    let mut seen: Vec<String> = vec![];
    let mut out: Vec<Preset> = Vec::with_capacity(list.len() + BUILTINS.len());
    for mut p in std::mem::take(list) {
        let link = p.builtin.clone();
        match link.as_deref().and_then(factory) {
            Some(f) if !seen.contains(&f.id) => {
                seen.push(f.id.clone());
                let copy = differs(&p).then(|| p.clone());
                let (pinned, category, hidden, id) = (p.pinned, p.category.clone(), p.hidden, p.id.clone());
                out.push(Preset { id, pinned, category, hidden, ..f.clone() });
                if let Some(mut c) = copy {
                    let base = if c.name.trim().eq_ignore_ascii_case(&f.name) || c.name.trim().is_empty() { format!("{} (your edits)", f.name) } else { c.name.trim().to_string() };
                    if c.values.is_null() {
                        c.values = f.values.clone();
                    }
                    if c.kind.trim().is_empty() {
                        c.kind = f.kind.clone();
                    }
                    c.id = new_id();
                    c.builtin = None;
                    c.hidden = false;
                    c.name = unique_name(&out, &base);
                    notes.push(format!("The built-in preset “{}” had your own changes. Built-ins are read-only now, so your version was kept as the preset “{}”.", f.name, c.name));
                    out.push(c);
                }
            }
            _ => {
                if link.is_some() {
                    p.builtin = None;
                }
                out.push(p);
            }
        }
    }
    // factory presets not in the file
    let known: Vec<&str> = if known.is_empty() { BUILTINS_V02.to_vec() } else { known.iter().map(|s| s.as_str()).collect() };
    for f in factory_all() {
        if !seen.contains(&f.id) {
            let deleted = known.contains(&f.id.as_str());
            out.push(Preset { hidden: deleted, ..f });
        }
    }
    // user presets: no factory ids, no duplicate ids, no built-in names
    let builtin_names: Vec<String> = out.iter().filter(|p| p.is_builtin()).map(|p| p.name.to_lowercase()).collect();
    let mut ids: Vec<String> = out.iter().filter(|p| p.is_builtin()).map(|p| p.id.clone()).collect();
    for i in 0..out.len() {
        if out[i].is_builtin() {
            continue;
        }
        if factory(&out[i].id).is_some() || ids.contains(&out[i].id) || out[i].id.trim().is_empty() {
            out[i].id = new_id();
        }
        ids.push(out[i].id.clone());
        if builtin_names.contains(&out[i].name.trim().to_lowercase()) {
            let old = out[i].name.clone();
            let others: Vec<Preset> = out.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, p)| p.clone()).collect();
            out[i].name = unique_name(&others, &old);
            notes.push(format!("Your preset “{old}” has the same name as a built-in and was renamed to “{}”.", out[i].name));
        }
    }
    *list = out;
    notes
}

/// Load presets; a missing file gives the factory list. A corrupt file is
/// renamed to `*.bad` (so it is never silently overwritten) and the factory list is used.
pub fn load_from(path: &Path) -> (Vec<Preset>, Option<String>) {
    let (l, notes) = load_from_with(path, &differs_raw);
    (l, notes.into_iter().find(|n| n.contains("unreadable")))
}

/// [`load_from`] with the app's "differs from factory" test; returns all notes.
pub fn load_from_with(path: &Path, differs: &dyn Fn(&Preset) -> bool) -> (Vec<Preset>, Vec<String>) {
    match std::fs::read_to_string(path) {
        Err(_) => (factory_all(), vec![]),
        Ok(t) => match parse_file(&t) {
            Ok((mut l, known)) => {
                let notes = enforce_builtins(&mut l, &known.unwrap_or_default(), differs);
                (l, notes)
            }
            Err(e) => {
                let bad = path.with_extension(format!("bad-{}", crate::util::now_unix()));
                let _ = std::fs::rename(path, &bad);
                (factory_all(), vec![format!("Presets file was unreadable ({e}); moved to {} and started from the built-ins.", bad.display())])
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


/// Form keys that hold the app-level input/output folders (not part of a preset).
pub const APP_FOLDER_FIELDS: &[&str] = &["out_dir", "in_dir", "src_dir"];

/// Drop input/output folder keys so presets describe only how to encode.
pub fn strip_folder_fields(v: Value) -> Value {
    strip_run_fields(v, APP_FOLDER_FIELDS)
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
    fn experimental_codec_presets() {
        use crate::cmd::codecs::VideoCodec;
        use crate::cmd::common::{AudioEncode, VideoEncode};
        let v = |id: &str| -> VideoEncode { serde_json::from_value(factory(id).unwrap().values["video"].clone()).unwrap() };
        let h = v("builtin-hevc-game");
        assert_eq!((h.codec, h.crf, h.preset.as_str(), h.ten_bit), (VideoCodec::Hevc, 20, "slow", true));
        assert_eq!(v("builtin-hevc-camera").crf, 18);
        let a = v("builtin-av1-game");
        assert_eq!((a.codec, a.crf, a.svt_preset, a.ten_bit, a.film_grain), (VideoCodec::Av1, 28, 5, true, 0));
        assert_eq!(v("builtin-av1-camera").crf, 26);
        // audio stays AAC default
        assert!(factory("builtin-av1-camera").unwrap().values.get("audio").is_none());
        assert_eq!(AudioEncode::default().codec, crate::cmd::codecs::AudioCodec::Aac);
        // plain re-encode stays H.264
        assert!(factory("builtin-reencode").unwrap().values.is_null());
        assert_eq!(VideoEncode::default().codec, VideoCodec::H264);
    }

    #[test]
    fn upgrade_adds_codec_presets_only() {
        // a 0.5.3 presets file: all old built-ins known, plus a user preset
        let old_ids: Vec<String> = BUILTINS.iter().filter(|b| !b.0.contains("hevc") && !b.0.contains("av1")).map(|b| b.0.to_string()).collect();
        let mut list: Vec<Preset> = old_ids.iter().map(|id| Preset { values: Value::Null, ..factory(id).unwrap() }).collect();
        let user = Preset { id: "p1".into(), name: "Mine".into(), builtin: None, values: json!({"video": {"crf": 17, "preset": "medium"}}), ..factory("builtin-reencode").unwrap() };
        list.push(user.clone());
        let mut out = list;
        enforce_builtins(&mut out, &old_ids, &|_| false);
        for id in ["builtin-hevc-game", "builtin-hevc-camera", "builtin-av1-game", "builtin-av1-camera"] {
            let p = out.iter().find(|p| p.id == id).expect(id);
            assert!(!p.hidden && p.category == EXPERIMENTAL_GROUP);
        }
        let u = out.iter().find(|p| p.id == "p1").unwrap();
        assert_eq!((u.name.as_str(), &u.values, u.builtin.is_none()), ("Mine", &user.values, true));
        assert_eq!(out.len(), BUILTINS.len() + 1);
    }

    #[test]
    fn factory_list() {
        let f = factory_all();
        assert_eq!(f.len(), BUILTINS.len());
        assert!(f.iter().all(|p| p.builtin.as_deref() == Some(p.id.as_str())));
        assert!(f.iter().filter(|p| p.id != "builtin-addchapters" && p.category.is_empty()).all(|p| p.values.is_null()));
        assert_eq!(f.iter().filter(|p| p.category == EXPERIMENTAL_GROUP).count(), 4);
        assert_eq!(factory("builtin-addchapters").unwrap().values["container"], "same");
        let ids: std::collections::HashSet<_> = f.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids.len(), f.len());
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
        l.push(Preset { id: "p1".into(), name: "Mine".into(), builtin: None, values: json!({"video": {"crf": 18}}), ..factory("builtin-reencode").unwrap() });
        l[2].pinned = true;
        l[3].hidden = true;
        save_to(&path, &l).unwrap();
        assert_eq!(load_from(&path), (l.clone(), None));
        // built-ins are stored without values (they come from the app code)
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(raw["presets"].as_array().unwrap().iter().filter(|p| p.get("builtin").is_some()).all(|p| p["values"].is_null()));
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
        let b = Preset { id: "zzz".into(), name: "Mine".into(), kind: "custom".into(), description: String::new(), builtin: None, values: Value::Null, pinned: false, category: String::new(), hidden: false };
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
    fn new_builtins_appended_deleted_ones_come_back_hidden() {
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
        let get = |id: &str| l.iter().find(|p| p.id == id).unwrap().clone();
        assert!(!get("builtin-addchapters").hidden, "new built-in appended, visible");
        assert!(get("builtin-mix").hidden, "deleted built-in is back, but hidden");
        assert!(!get("builtin-frameserve").hidden);
        assert!(l[0].pinned && l[0].category == "Daily");
        assert_eq!(l.iter().filter(|p| p.is_builtin()).count(), BUILTINS.len());
        // after saving, a hidden built-in stays hidden
        save_to(&path, &l).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("known_builtins"));
        assert!(!text.contains("\"pinned\": false") && !text.contains("\"hidden\": false"), "defaults not written");
        assert_eq!(load_from(&path).0, l);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn edited_builtins_migrate_to_user_presets() {
        let mut re = factory("builtin-reencode").unwrap();
        re.values = json!({"video": {"crf": 18}});
        re.pinned = true;
        let mut mix = factory("builtin-mix").unwrap();
        mix.name = "My mix".into();
        mix.category = "Audio".into();
        let mut dup = factory("builtin-still").unwrap();
        dup.id = "p-dup".into();
        dup.name = "Still copy".into();
        // user preset that uses a factory id and a built-in's name
        let squat = Preset { id: "builtin-join".into(), name: "join / STITCH files".into(), builtin: None, ..factory("builtin-custom").unwrap() };
        let gone = Preset { id: "builtin-old".into(), builtin: Some("builtin-old".into()), name: "Old".into(), ..factory("builtin-custom").unwrap() };
        let mut l = vec![re, mix, factory("builtin-still").unwrap(), dup, squat, gone];
        let known: Vec<String> = BUILTINS.iter().map(|b| b.0.to_string()).collect();
        let notes = enforce_builtins(&mut l, &known, &differs_raw);
        let by_name = |n: &str| l.iter().find(|p| p.name == n).unwrap_or_else(|| panic!("no {n}: {:?}", l.iter().map(|p| &p.name).collect::<Vec<_>>())).clone();
        // the built-in itself is factory again, keeping pin / folder
        let b = l.iter().find(|p| p.id == "builtin-reencode").unwrap();
        assert!(b.values.is_null() && b.name == "Re-encode video" && b.pinned);
        // the user's version is right after it
        let pos = l.iter().position(|p| p.id == "builtin-reencode").unwrap();
        assert_eq!(l[pos + 1].name, "Re-encode video (your edits)");
        assert_eq!(l[pos + 1].values["video"]["crf"], 18);
        assert!(l[pos + 1].builtin.is_none() && l[pos + 1].id.starts_with('p'));
        // renamed built-in: copy keeps the user's name, built-in gets its own back
        assert_eq!(by_name("Mix audio tracks to one (MKV)").category, "Audio");
        let m = by_name("My mix");
        assert!(m.builtin.is_none() && m.kind == "mix" && m.category == "Audio");
        // duplicate link -> plain user preset with its own id
        let d = by_name("Still copy");
        assert!(d.builtin.is_none() && d.id == "p-dup");
        // squatter: new id, renamed away from the built-in name
        let s = l.iter().find(|p| p.name.starts_with("join / STITCH files")).unwrap();
        assert!(s.id != "builtin-join" && s.name == "join / STITCH files (2)", "{}", s.name);
        // unknown built-in link -> user preset
        assert!(by_name("Old").builtin.is_none());
        // every factory preset exactly once, ids unique
        assert_eq!(l.iter().filter(|p| p.is_builtin()).count(), BUILTINS.len());
        let ids: std::collections::HashSet<_> = l.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids.len(), l.len());
        assert!(notes.iter().any(|n| n.contains("Re-encode video (your edits)")));
        assert!(notes.iter().any(|n| n.contains("renamed")));
        // running it again changes nothing
        let again = l.clone();
        assert!(enforce_builtins(&mut l, &known, &differs_raw).is_empty());
        assert_eq!(l, again);
    }

    #[test]
    fn paths() {
        assert_eq!(presets_path_for(&Path::new("/x").join(crate::settings::PORTABLE_FILE)), PathBuf::from("/x").join(PORTABLE_PRESETS_FILE));
        assert_eq!(presets_path_for(Path::new("/r/FFmpegEncoder/settings.json")), PathBuf::from("/r/FFmpegEncoder/presets.json"));
    }
}
