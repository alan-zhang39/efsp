//! Reads Npcap facts from the system. Only checks that files and registry
//! keys exist; it never loads the driver or the DLL.

use efsp_core::npcap::NpcapStatus;

#[cfg(windows)]
pub fn status() -> NpcapStatus {
    efsp_core::npcap::interpret(&win::probe())
}

#[cfg(not(windows))]
pub fn status() -> NpcapStatus {
    NpcapStatus::NotApplicable
}

#[cfg(windows)]
mod win {
    use efsp_core::npcap::NpcapProbe;
    use std::path::PathBuf;
    use windows_registry::LOCAL_MACHINE;

    // The Npcap installer is 32-bit, so its keys may sit under WOW6432Node
    // when read from this 64-bit app. Check both views.
    const INSTALL_KEYS: [&str; 2] = [r"SOFTWARE\Npcap", r"SOFTWARE\WOW6432Node\Npcap"];
    const UNINSTALL_KEYS: [&str; 2] = [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\NpcapInst",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\NpcapInst",
    ];

    // Set to 1 when Npcap was installed with "Restrict driver access to
    // Administrators only". Verified on Npcap 1.88.
    const PARAMETERS_KEY: &str = r"SYSTEM\CurrentControlSet\Services\npcap\Parameters";

    pub fn probe() -> NpcapProbe {
        NpcapProbe {
            dll_present: dll_path().is_file(),
            registry_present: INSTALL_KEYS.iter().any(|k| LOCAL_MACHINE.open(k).is_ok()),
            version: UNINSTALL_KEYS.iter().find_map(|k| {
                LOCAL_MACHINE
                    .open(k)
                    .ok()
                    .and_then(|key| key.get_string("DisplayVersion").ok())
            }),
            admin_only: LOCAL_MACHINE
                .open(PARAMETERS_KEY)
                .and_then(|key| key.get_u32("AdminOnly"))
                .is_ok_and(|v| v == 1),
        }
    }

    fn dll_path() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(root).join(r"System32\Npcap\wpcap.dll")
    }
}
