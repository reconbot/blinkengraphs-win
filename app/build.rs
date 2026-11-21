// Windows 10+ compatibility in the manifest is what allows layered child windows (the Taskbar Strip).
fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::{embed_manifest, manifest::DpiAwareness, new_manifest};
        embed_manifest(new_manifest("blinkengraphs").dpi_awareness(DpiAwareness::PerMonitorV2))
            .expect("manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
