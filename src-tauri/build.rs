fn main() {
    // D37: replace tauri-build's default manifest with one that declares DPI
    // awareness. The default has no <dpiAware> element, so awareness is left to
    // tao's silent fallback ladder. See windows-app-manifest.xml.
    let windows = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("windows-app-manifest.xml"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
