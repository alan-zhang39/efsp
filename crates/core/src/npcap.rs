//! Npcap detection.
//!
//! The Tauri app gathers raw facts from Windows (does the DLL exist, what does
//! the registry say) into an [`NpcapProbe`]; [`interpret`] turns them into a
//! status for the UI. Keeping the decision here makes it testable on macOS.

use serde::Serialize;

/// Official installer page. The app opens this in the user's browser; it
/// never downloads or runs the installer itself.
pub const NPCAP_DOWNLOAD_URL: &str = "https://npcap.com/#download";

/// Raw facts read from the system. All fields default to "nothing found".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NpcapProbe {
    /// `%SystemRoot%\System32\Npcap\wpcap.dll` exists.
    pub dll_present: bool,
    /// Npcap's install key exists in the registry.
    pub registry_present: bool,
    /// Version string from the uninstall entry, if any.
    pub version: Option<String>,
    /// Npcap was installed with "Restrict driver access to Administrators
    /// only", so this unelevated app can't capture.
    pub admin_only: bool,
}

/// What the UI shows. Serialised as `{ "state": "found", "version": "1.80" }`
/// etc., mirrored by `NpcapStatus` in `src/lib/npcap.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum NpcapStatus {
    /// The capture DLL is present and usable without admin rights.
    Found { version: Option<String> },
    /// Installed, but restricted to administrators: the user must reinstall
    /// with that option unticked (the app itself never runs elevated).
    Restricted { version: Option<String> },
    /// Not usable. `partial` means the registry says installed but the DLL is
    /// gone, so the user should reinstall rather than install.
    Missing { partial: bool },
    /// Not Windows: live capture is unavailable; development uses replay.
    NotApplicable,
}

/// Decides the status from a Windows probe. The DLL is the source of truth:
/// it is what capture will actually load.
pub fn interpret(probe: &NpcapProbe) -> NpcapStatus {
    if probe.dll_present {
        let version = probe
            .version
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        if probe.admin_only {
            NpcapStatus::Restricted { version }
        } else {
            NpcapStatus::Found { version }
        }
    } else {
        NpcapStatus::Missing {
            partial: probe.registry_present,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_installed_is_missing() {
        assert_eq!(
            interpret(&NpcapProbe::default()),
            NpcapStatus::Missing { partial: false }
        );
    }

    #[test]
    fn dll_present_is_found_with_trimmed_version() {
        let probe = NpcapProbe {
            dll_present: true,
            registry_present: true,
            version: Some(" 1.80 ".into()),
            admin_only: false,
        };
        assert_eq!(
            interpret(&probe),
            NpcapStatus::Found {
                version: Some("1.80".into())
            }
        );
    }

    #[test]
    fn blank_version_becomes_none() {
        let probe = NpcapProbe {
            dll_present: true,
            version: Some("  ".into()),
            ..Default::default()
        };
        assert_eq!(interpret(&probe), NpcapStatus::Found { version: None });
    }

    #[test]
    fn admin_only_install_is_restricted() {
        let probe = NpcapProbe {
            dll_present: true,
            admin_only: true,
            version: Some("1.80".into()),
            ..Default::default()
        };
        assert_eq!(
            interpret(&probe),
            NpcapStatus::Restricted {
                version: Some("1.80".into())
            }
        );
    }

    #[test]
    fn registry_without_dll_is_partial_install() {
        let probe = NpcapProbe {
            registry_present: true,
            ..Default::default()
        };
        assert_eq!(interpret(&probe), NpcapStatus::Missing { partial: true });
    }

    #[test]
    fn serialises_to_the_shape_the_ui_expects() {
        let json = |s: NpcapStatus| serde_json::to_string(&s).unwrap();
        assert_eq!(
            json(NpcapStatus::Found {
                version: Some("1.80".into())
            }),
            r#"{"state":"found","version":"1.80"}"#
        );
        assert_eq!(
            json(NpcapStatus::Missing { partial: false }),
            r#"{"state":"missing","partial":false}"#
        );
        assert_eq!(
            json(NpcapStatus::Restricted { version: None }),
            r#"{"state":"restricted","version":null}"#
        );
        assert_eq!(
            json(NpcapStatus::NotApplicable),
            r#"{"state":"notApplicable"}"#
        );
    }
}
