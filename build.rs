// Embeds the Windows icon + manifest + VERSIONINFO when building for a Windows target.
// FileVersion / ProductVersion are generated from CARGO_PKG_VERSION so they stay in sync
// with the crate (0.6.0-rc.3 → 0.6.0.3). Never fails the build: if the resource compiler
// is missing we just skip it.
include!("src/winver.rs");

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") || !std::path::Path::new("assets/app.rc").exists() {
        return;
    }
    let pkg = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let commas = file_version_commas(&pkg);
    let dotted = file_version_string(&pkg);
    let src = std::fs::read_to_string("assets/app.rc").expect("assets/app.rc");
    let mut out = String::new();
    for line in src.lines() {
        let t = line.trim_start();
        let line = if t.starts_with("FILEVERSION") {
            format!("FILEVERSION {commas}")
        } else if t.starts_with("PRODUCTVERSION") {
            format!("PRODUCTVERSION {commas}")
        } else if t.contains("\"FileVersion\"") {
            format!("      VALUE \"FileVersion\", \"{dotted}\"")
        } else if t.contains("\"ProductVersion\"") {
            format!("      VALUE \"ProductVersion\", \"{dotted}\"")
        } else {
            line.to_string()
        };
        out.push_str(&line);
        out.push('\n');
    }
    let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.rc");
    std::fs::write(&dest, out).expect("write generated app.rc");
    let _ = embed_resource::compile(&dest, embed_resource::NONE).manifest_optional();
}
