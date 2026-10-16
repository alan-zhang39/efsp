fn main() {
    // On Windows, load Npcap's wpcap.dll lazily (on first use) instead of at
    // startup. Without this the app couldn't even open on a PC without Npcap,
    // so it could never show the "Install Npcap" screen. live_source.rs points
    // the DLL search path at System32\Npcap before the first pcap call.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg=/DELAYLOAD:wpcap.dll");
        println!("cargo:rustc-link-lib=delayimp");
    }

    // Declaring the app's commands here makes Tauri generate one permission
    // per command (`allow-npcap-status`, ...). The UI can then only call the
    // commands a capability file grants, nothing else.
    const COMMANDS: &[&str] = &[
        "npcap_status",
        "open_npcap_download",
        "history_list",
        "history_detail",
        "history_export_csv",
        "history_delete_all",
        "settings_get",
        "settings_set",
    ];
    println!("cargo:rerun-if-changed=capabilities");

    let manifest = tauri_build::AppManifest::new().commands(COMMANDS);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri-build");
}
