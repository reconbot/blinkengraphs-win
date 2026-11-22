// Windows 10+ compatibility in the manifest is what allows layered child windows (the Taskbar Strip).
// The icon and manifest go in one resource script: two separate .rsrc inputs don't link cleanly.
fn main() {
    let svg = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/brand/icon.svg");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", svg.display());
    if std::env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }
    use embed_manifest::{manifest::DpiAwareness, new_manifest};
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let manifest = new_manifest("blinkengraphs").dpi_awareness(DpiAwareness::PerMonitorV2);
    std::fs::write(out.join("app.manifest"), manifest.to_string()).unwrap();
    let svg = std::fs::read(&svg).expect("spec/brand/icon.svg: run git submodule update --init");
    std::fs::write(out.join("app.ico"), brand::Icon::parse(&svg).ico()).unwrap();
    let rc = out.join("app.rc");
    std::fs::write(&rc, "1 ICON \"app.ico\"\n1 24 \"app.manifest\"\n").unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
