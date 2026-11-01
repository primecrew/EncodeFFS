// Embeds the Windows icon + manifest when building for a Windows target.
// Never fails the build: if the resource compiler is missing we just skip it.
fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") && std::path::Path::new("assets/app.rc").exists() {
        let _ = embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional();
    }
}
