fn main() {
    // Own manifest: asInvoker (no admin rights), PerMonitorV2 DPI awareness, long paths, Windows 10/11.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("d256shell.manifest"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
