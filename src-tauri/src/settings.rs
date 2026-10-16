//! User settings, saved as JSON in efsp's config folder.
//!
//! Unknown or missing fields fall back to defaults, so older settings files
//! keep working as new settings are added.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    /// Dark is the default (gamers play in dark rooms).
    #[default]
    Dark,
    Light,
    System,
}

/// The Live page's card: ping on the left with details (L1), or one big
/// centred ping (L2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LiveLayout {
    #[default]
    Card,
    Focus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HistoryView {
    #[default]
    Rows,
    Cards,
}

/// Which figures and graph lines to show.
/// Ping and bars are always shown: they are the card. Packets/s is off by
/// default: jitter matters, incoming packets don't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Show {
    pub jitter: bool,
    pub loss: bool,
    pub packets: bool,
    pub graph_jitter: bool,
    pub graph_packets: bool,
    pub graph_losses: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            jitter: true,
            loss: true,
            packets: false,
            graph_jitter: true,
            graph_packets: false,
            graph_losses: true,
        }
    }
}

/// How long to keep history. `None` = forever.
pub const KEEP_DAYS_OPTIONS: [Option<u32>; 4] = [None, Some(30), Some(90), Some(365)];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    pub start_with_windows: bool,
    /// When started with Windows, stay in the tray instead of opening.
    pub start_minimised: bool,
    pub show: Show,
    pub history_view: HistoryView,
    /// Rows or cards on the Servers page.
    pub servers_view: HistoryView,
    pub live_layout: LiveLayout,
    pub keep_history_days: Option<u32>,
}

impl Settings {
    /// Rejects values the UI can't produce, so a hand-edited or tampered
    /// file can't set something unexpected.
    pub fn validate(&self) -> Result<(), String> {
        if !KEEP_DAYS_OPTIONS.contains(&self.keep_history_days) {
            return Err("Keep history must be 30, 90 or 365 days, or forever.".into());
        }
        Ok(())
    }
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

/// Loads settings; a missing, unreadable or invalid file gives the defaults.
pub fn load(config_dir: &Path) -> Settings {
    std::fs::read_to_string(path(config_dir))
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .filter(|s| s.validate().is_ok())
        .unwrap_or_default()
}

/// Saves settings atomically: write a temp file, then rename over the old
/// one, so a crash mid-write can't leave a half-written file.
pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    settings.validate()?;
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = config_dir.join("settings.json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path(config_dir)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("efsp-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_gives_defaults() {
        let s = load(&temp_dir("missing"));
        assert_eq!(s, Settings::default());
        assert_eq!(s.theme, Theme::Dark);
        assert!(s.show.jitter && !s.show.packets && !s.start_with_windows);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = temp_dir("roundtrip");
        let s = Settings {
            theme: Theme::System,
            show: Show {
                graph_packets: true,
                ..Show::default()
            },
            history_view: HistoryView::Cards,
            keep_history_days: Some(90),
            ..Settings::default()
        };
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir), s);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partial_old_files_fill_in_defaults_and_bad_files_are_ignored() {
        let dir = temp_dir("partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), r#"{"theme":"light"}"#).unwrap();
        let s = load(&dir);
        assert_eq!(s.theme, Theme::Light);
        assert!(s.show.jitter, "missing fields default");

        std::fs::write(path(&dir), "not json").unwrap();
        assert_eq!(load(&dir), Settings::default());
        std::fs::write(path(&dir), r#"{"keepHistoryDays":7}"#).unwrap();
        assert_eq!(load(&dir), Settings::default(), "invalid value rejected");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn validate_rejects_odd_retention() {
        let s = Settings {
            keep_history_days: Some(1),
            ..Default::default()
        };
        assert!(s.validate().is_err());
        assert!(save(&temp_dir("bad"), &s).is_err());
    }
}
