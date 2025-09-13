use std::{env, fs, path::PathBuf};

fn main() {
    // Ensure tauri build steps still run
    tauri_build::build();

    // Try to keep icons/icon.png in sync with our branded logo
    // so dev/builds reflect the latest branding without manual steps.
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let icons_dir = PathBuf::from(manifest_dir).join("icons");
    let src = icons_dir.join("ishinex-logo.png");
    let dest = icons_dir.join("icon.png");

    // Re-run build script if the source logo changes
    println!("cargo:rerun-if-changed={}", src.display());

    if src.exists() {
        // Best-effort copy; ignore errors to avoid breaking builds
        let _ = fs::copy(&src, &dest);
    }
}
