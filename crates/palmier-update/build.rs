// Embed an `asInvoker` application manifest on Windows MSVC.
//
// WHY: Windows installer-detection heuristics auto-flag any executable whose
// name contains "update"/"setup"/"install" as requiring elevation (UAC). That
// makes `cargo test` for the `palmier-update` crate fail to launch its test
// binary with "The requested operation requires elevation. (os error 740)".
// Embedding a manifest that requests `asInvoker` disables the heuristic.
//
// This affects only this crate's own test/example binaries (the manifest is
// linked into binaries built FROM this crate). The real app binary is
// `palmier-tauri`, which is unaffected and will ship its own Tauri manifest.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("palmier-update.manifest");
        println!("cargo:rerun-if-changed=palmier-update.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
