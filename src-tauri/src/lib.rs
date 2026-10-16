mod geo;
mod npcap_probe;
mod settings;
mod store;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use efsp_core::history::{MatchRecord, to_csv};
use efsp_core::npcap::{NPCAP_DOWNLOAD_URL, NpcapStatus};
use efsp_core::session::Point;
use serde::{Deserialize, Serialize};
use settings::Settings;
use store::{History, ListQuery};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// Passed by Windows when efsp starts with the user's sign-in.
const AUTOSTART_ARG: &str = "--autostart";

/// App-wide state opened at startup.
struct AppState {
    /// `None` if the history database couldn't be opened; the rest of the
    /// app still works.
    history: Option<Arc<History>>,
    settings: Mutex<Settings>,
    config_dir: PathBuf,
    /// Offline country/network lookups for classifying remotes.
    #[expect(
        dead_code,
        reason = "used by the monitor, which arrives with the capture pipeline"
    )]
    geo: geo::Geo,
}

fn history_of(state: &AppState) -> Result<&Arc<History>, String> {
    state
        .history
        .as_ref()
        .ok_or_else(|| "Match history couldn't be opened on this PC.".to_owned())
}

/// Reports whether Npcap (the capture driver) is installed. `async` runs it
/// off the main thread, so registry and disk reads never freeze the window.
#[tauri::command]
async fn npcap_status() -> NpcapStatus {
    npcap_probe::status()
}

/// Opens the official Npcap download page in the default browser. The URL is
/// fixed here, so the UI cannot ask the app to open anything else.
#[tauri::command]
fn open_npcap_download(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(NPCAP_DOWNLOAD_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryListArgs {
    /// Only the last N days; `None` for everything.
    since_days: Option<u32>,
    include_offers: bool,
    limit: u32,
    offset: u32,
    /// "server" or "peer" to show only those.
    #[serde(default)]
    kind: Option<String>,
    /// Only servers in cities whose name contains this.
    #[serde(default)]
    city: Option<String>,
    /// "when" (default), "ping" or "length".
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    ascending: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryPage {
    records: Vec<MatchRecord>,
    total: u32,
}

/// One page of saved matches, newest first. Numbers only, no addresses.
#[tauri::command]
fn history_list(state: State<'_, AppState>, query: HistoryListArgs) -> Result<HistoryPage, String> {
    let history = history_of(&state)?;
    let since = query
        .since_days
        .map(|d| unix_now_s() - i64::from(d) * 86_400);
    let city = query
        .city
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let q = ListQuery {
        since,
        include_offers: query.include_offers,
        limit: query.limit.min(500),
        offset: query.offset,
        endpoint_kind: match query.kind.as_deref() {
            Some("server") => Some("server"),
            Some("peer") => Some("peer"),
            _ => None,
        },
        regions: city.map(|c| {
            efsp_core::classify::regions_matching(c)
                .into_iter()
                .map(str::to_owned)
                .collect()
        }),
        sort: match query.sort.as_deref() {
            Some("ping") => store::Sort::Ping,
            Some("length") => store::Sort::Length,
            _ => store::Sort::When,
        },
        ascending: query.ascending,
    };
    history.with(|s| {
        let records = s.list(&q).map_err(|e| e.to_string())?;
        let total = s.count(&q).map_err(|e| e.to_string())?;
        Ok(HistoryPage { records, total })
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryDetail {
    record: MatchRecord,
    timeline: Vec<Point>,
}

/// One saved match with its whole timeline.
#[tauri::command]
fn history_detail(state: State<'_, AppState>, id: i64) -> Result<Option<HistoryDetail>, String> {
    history_of(&state)?.with(|s| {
        s.detail(id)
            .map(|found| found.map(|(record, timeline)| HistoryDetail { record, timeline }))
            .map_err(|e| e.to_string())
    })
}

/// Saves the whole history as CSV where the user picks, via Windows' own
/// "Save as" dialog. No fingerprints, no addresses. Returns false if the
/// user cancelled.
#[tauri::command]
async fn history_export_csv(app: AppHandle) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let history = history_of(&state)?;
        let all = ListQuery {
            include_offers: true,
            limit: u32::MAX,
            ..ListQuery::default()
        };
        let records = history.with(|s| s.list(&all)).map_err(|e| e.to_string())?;
        let Some(target) = app
            .dialog()
            .file()
            .add_filter("CSV", &["csv"])
            .set_file_name("efsp-history.csv")
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = target.into_path().map_err(|e| e.to_string())?;
        std::fs::write(path, to_csv(&records)).map_err(|e| e.to_string())?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes every saved match after a native confirmation dialog, so nothing
/// in the page can wipe history without the user's click. Returns how many
/// were deleted, or `None` if the user cancelled.
#[tauri::command]
async fn history_delete_all(app: AppHandle) -> Result<Option<usize>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let history = history_of(&state)?;
        let count = history.with(|s| s.count_all()).map_err(|e| e.to_string())?;
        let mut dialog = app
            .dialog()
            .message(format!(
                "Delete all {count} saved matches and offers? This can't be undone."
            ))
            .title("Delete match history");
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.parent(&window); // modal to efsp's window
        }
        let confirmed = dialog
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Delete all".into(),
                "Cancel".into(),
            ))
            .blocking_show();
        if !confirmed {
            return Ok(None);
        }
        history
            .with(|s| s.delete_all())
            .map(Some)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn settings_get(state: State<'_, AppState>) -> Settings {
    state
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Validates, applies, then saves settings. Shortening "keep history" in a
/// way that deletes matches asks first in a native dialog. Returns what is
/// now in effect (the old settings if the user cancelled).
#[tauri::command]
async fn settings_set(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        settings.validate()?;
        let previous = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if !confirm_retention(&app, &state, &previous, &settings) {
            return Ok(previous);
        }
        // Apply first: if Windows refuses the startup entry, nothing is saved.
        apply_settings(&app, &state, &settings)?;
        settings::save(&state.config_dir, &settings)?;
        *state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = settings.clone();
        Ok(settings)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// True to go ahead. Asks only when the new retention would delete records.
fn confirm_retention(app: &AppHandle, state: &AppState, old: &Settings, new: &Settings) -> bool {
    let (Some(days), Some(history)) = (new.keep_history_days, &state.history) else {
        return true;
    };
    if old.keep_history_days.is_some_and(|d| d <= days) {
        return true; // not shorter than before: nothing new to delete
    }
    let before = unix_now_s() - i64::from(days) * 86_400;
    let n = history.with(|s| s.count_before(before)).unwrap_or(0);
    if n == 0 {
        return true;
    }
    let mut dialog = app
        .dialog()
        .message(format!(
            "Keeping {days} days of history removes {n} older saved matches and offers now. \
             This can't be undone."
        ))
        .title("Shorten match history")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Remove them".into(),
            "Cancel".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.blocking_show()
}

/// Side effects of settings that live outside the app.
fn apply_settings(app: &AppHandle, state: &AppState, settings: &Settings) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let enabled = autolaunch.is_enabled().unwrap_or(false);
    if settings.start_with_windows && !enabled {
        autolaunch.enable().map_err(|e| e.to_string())?;
    } else if !settings.start_with_windows && enabled {
        autolaunch.disable().map_err(|e| e.to_string())?;
    }
    apply_window_theme(app, settings);
    prune_history(state, settings);
    Ok(())
}

/// The window frame (title bar) follows the app theme; System follows Windows.
fn apply_window_theme(app: &AppHandle, settings: &Settings) {
    if let Some(window) = app.get_webview_window("main") {
        let theme = match settings.theme {
            settings::Theme::Dark => Some(tauri::Theme::Dark),
            settings::Theme::Light => Some(tauri::Theme::Light),
            settings::Theme::System => None,
        };
        let _ = window.set_theme(theme);
    }
}

fn prune_history(state: &AppState, settings: &Settings) {
    if let (Some(days), Some(history)) = (settings.keep_history_days, &state.history) {
        let before = unix_now_s() - i64::from(days) * 86_400;
        let _ = history.with(|s| s.prune(before));
    }
}

fn unix_now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch (e.g. from the Start menu while
        // efsp sits in the tray) just shows this window instead of starting
        // a second monitor that would save every match twice.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let settings = settings::load(&config_dir);
            let history = match History::open(&data_dir) {
                Ok(h) => Some(Arc::new(h)),
                Err(e) => {
                    // No addresses in this message; history just stays off.
                    eprintln!("efsp: match history unavailable: {e}");
                    None
                }
            };
            // Bundled under `ipdb/` (tauri.conf.json `bundle.resources`).
            let geo = match app.path().resource_dir() {
                Ok(dir) => geo::open(&dir.join("ipdb")),
                Err(e) => {
                    eprintln!("efsp: no resource folder, locations limited: {e}");
                    Arc::new(efsp_core::classify::NoGeo)
                }
            };
            let state = AppState {
                history,
                settings: Mutex::new(settings.clone()),
                config_dir,
                geo,
            };
            prune_history(&state, &settings);
            app.manage(state);

            apply_window_theme(app.handle(), &settings);

            // A tray problem mustn't stop the app; without a tray the window
            // always opens, so it can never be stuck hidden.
            let has_tray = match tray::build(app.handle()) {
                Ok(built) => built,
                Err(e) => {
                    eprintln!("efsp: tray icon unavailable: {e}");
                    false
                }
            };

            // Started by Windows with "start in the tray" on: stay hidden.
            let autostarted = std::env::args().any(|a| a == AUTOSTART_ARG);
            let stay_in_tray = has_tray && autostarted && settings.start_minimised;
            if !stay_in_tray && let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// The tray icon: lets efsp run in the background and reopen with a click.
mod tray {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, Manager};

    pub fn show_main(app: &AppHandle) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }

    /// Builds the tray icon. `Ok(false)` if there's no icon image to show,
    /// in which case there's no usable tray.
    pub fn build(app: &AppHandle) -> tauri::Result<bool> {
        let Some(icon) = app.default_window_icon().cloned() else {
            return Ok(false);
        };
        let open = MenuItem::with_id(app, "open", "Open efsp", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open, &quit])?;
        let tray = TrayIconBuilder::with_id("efsp")
            .tooltip("efsp")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "open" => show_main(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main(tray.app_handle());
                }
            });
        tray.icon(icon).build(app)?;
        Ok(true)
    }
}

fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        npcap_status,
        open_npcap_download,
        history_list,
        history_detail,
        history_export_csv,
        history_delete_all,
        settings_get,
        settings_set
    ]
}
