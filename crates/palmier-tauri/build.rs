//! Tauri build script (E1-S1).
//!
//! `tauri_build` generates the Tauri context, embeds resources, and on
//! `windows-msvc` embeds the application manifest. We supply an **explicit**
//! manifest (`app.manifest`) so `requestedExecutionLevel level="asInvoker"` is
//! declared — the binary is NOT auto-flagged for UAC elevation by Windows'
//! installer-detection heuristic (signals/windows-uac-installer-detection.md).
//! Tauri's default manifest omits an explicit execution level; ours states it.
fn main() {
    let mut attrs = tauri_build::Attributes::new();

    #[cfg(windows)]
    {
        let manifest = std::fs::read_to_string("app.manifest")
            .expect("app.manifest must exist for the Windows asInvoker declaration");
        attrs = attrs.windows_attributes(
            tauri_build::WindowsAttributes::new().app_manifest(manifest),
        );
        // Rebuild if the manifest changes.
        println!("cargo:rerun-if-changed=app.manifest");
    }

    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}
