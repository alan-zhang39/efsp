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
    //
    // Developer-tool commands exist only with the `dev-tools` feature, and
    // so do their permissions: capabilities-dev/ is read only then.
    const RELEASE: &[&str] = &[
        "npcap_status",
        "open_npcap_download",
        "start_monitor",
        "history_list",
        "history_detail",
        "history_export_csv",
        "history_delete_all",
        "picker_status",
        "picker_apply",
        "picker_remove_all",
        "picker_choose_exe",
        "settings_get",
        "settings_set",
    ];
    const DEV: &[&str] = &[
        "npcap_status",
        "open_npcap_download",
        "start_monitor",
        "history_list",
        "history_detail",
        "history_export_csv",
        "history_delete_all",
        "picker_status",
        "picker_apply",
        "picker_remove_all",
        "picker_choose_exe",
        "settings_get",
        "settings_set",
        "dev_seed_history",
        "set_recording",
        "open_traces_folder",
        "compare_with_ping",
        "compare_game_pings",
        "test_route_probe",
    ];
    let dev_tools = std::env::var_os("CARGO_FEATURE_DEV_TOOLS").is_some();
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=capabilities-dev");

    let manifest = tauri_build::AppManifest::new().commands(if dev_tools { DEV } else { RELEASE });
    let mut attributes = tauri_build::Attributes::new().app_manifest(manifest);
    if dev_tools {
        // Matches capabilities/ and capabilities-dev/.
        attributes = attributes.capabilities_path_pattern("./capabilities*/**/*");
    }
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
