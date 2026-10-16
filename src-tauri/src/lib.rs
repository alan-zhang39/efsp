mod geo;
mod icmp;
mod live_source;
mod monitor;
mod npcap_probe;
mod picker;
mod port_watcher;
mod sample;
mod settings;
mod store;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use efsp_core::history::{MatchRecord, to_csv};
use efsp_core::npcap::{NPCAP_DOWNLOAD_URL, NpcapStatus};
use efsp_core::session::Point;
use monitor::{MonitorState, Snapshot};
use serde::{Deserialize, Serialize};
use settings::Settings;
use store::{History, ListQuery};
use tauri::ipc::Channel;
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

/// Starts live capture (Windows) or replay (`EFSP_REPLAY`), streaming
/// snapshots to `on_update`. Safe to call again after a page reload.
#[tauri::command]
fn start_monitor(
    app: AppHandle,
    state: State<'_, MonitorState>,
    app_state: State<'_, AppState>,
    on_update: Channel<Snapshot>,
) -> Result<(), String> {
    state.start(
        on_update,
        traces_dir(&app)?,
        app_state.history.clone(),
        Arc::clone(&app_state.geo),
    );
    Ok(())
}

/// What the Servers page needs to show the region picker.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickerStatus {
    /// Cities switched off by efsp's rules (empty if none or unknown).
    off: Vec<efsp_core::picker::Area>,
    /// efsp's rules are in Windows Firewall right now.
    rules_on: bool,
    /// The eFootball.exe rules will be scoped to, if found.
    game_exe: Option<String>,
    /// This build can change the firewall (Windows, helper installed).
    available: bool,
    /// A match is being measured: Apply must wait.
    match_active: bool,
}

fn picker_status_now(state: &AppState, monitor: &MonitorState) -> PickerStatus {
    let mut saved = picker::load(&state.config_dir);
    let before = saved.clone();
    let present = picker::rules_present();
    // The rules were removed outside efsp: forget them.
    if present == Some(false) {
        saved.off.clear();
    }
    // The game moved or was uninstalled: look again.
    if saved
        .game_exe
        .as_deref()
        .is_some_and(|exe| !picker::is_game_exe(std::path::Path::new(exe)))
    {
        saved.game_exe = None;
    }
    if saved.game_exe.is_none()
        && let Some(found) = picker::find_game_exe()
    {
        saved.game_exe = Some(found.to_string_lossy().into_owned());
    }
    if saved != before {
        let _ = picker::save(&state.config_dir, &saved);
    }
    PickerStatus {
        rules_on: present.unwrap_or(!saved.off.is_empty()),
        off: saved.off,
        game_exe: saved.game_exe,
        available: cfg!(windows) && picker::helper_path().is_some(),
        match_active: monitor.facts().match_active,
    }
}

/// One firewall change at a time (two UAC prompts would race on the
/// request file).
static PICKER_BUSY: Mutex<()> = Mutex::new(());

/// The guard rails for switching cities off, checked before the
/// confirmation and again right before the UAC prompt.
fn check_apply(facts: &monitor::LiveFacts, off: &[efsp_core::picker::Area]) -> Result<(), String> {
    if !facts.live {
        return Err(
            "Switching servers off needs live capture, so efsp can see when a match starts.".into(),
        );
    }
    // Never change rules under a live match: it could drop its packets.
    if facts.match_active {
        return Err("Finish the current match first: rules can't change during a match.".into());
    }
    if !facts.pinged_areas.is_empty()
        && facts
            .pinged_areas
            .iter()
            .all(|a| off.iter().any(|o| &o.key == a))
    {
        return Err("Leave at least one server the game is checking switched on.".into());
    }
    Ok(())
}

#[tauri::command]
async fn picker_status(app: AppHandle) -> Result<PickerStatus, String> {
    // Runs netsh and scans Steam folders: keep it off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        picker_status_now(&app.state::<AppState>(), &app.state::<MonitorState>())
    })
    .await
    .map_err(|e| e.to_string())
}

/// Switches these cities off (and every other city on) with one UAC prompt.
/// `Ok(None)` if the player cancelled.
#[tauri::command]
async fn picker_apply(
    app: AppHandle,
    off: Vec<efsp_core::picker::Area>,
) -> Result<Option<PickerStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(_busy) = PICKER_BUSY.try_lock() else {
            return Err("efsp is already changing the rules.".into());
        };
        let state = app.state::<AppState>();
        let monitor = app.state::<MonitorState>();
        if off.is_empty() {
            return remove_rules(&app);
        }
        check_apply(&monitor.facts(), &off)?;
        let mut saved = picker::load(&state.config_dir);
        let program = saved
            .game_exe
            .clone()
            .ok_or("Choose eFootball.exe first, so the rules only affect the game.")?;
        // Built here only to check it; the helper builds its own from the cities.
        efsp_core::picker::build_rules(&off, &program).map_err(|e| e.to_string())?;

        // One native confirmation with everything the player should know.
        let names: Vec<&str> = off.iter().map(|a| a.name.as_str()).collect();
        let usual = history_of(&state)
            .ok()
            .and_then(|h| h.with(|s| s.server_rows()).ok())
            .map(|rows| efsp_core::history::region_stats(&rows))
            .and_then(|stats| stats.into_iter().find(|r| r.matches > 0))
            .map(|r| r.endpoint);
        let usual_off = usual.as_ref().is_some_and(|u| {
            let key = u.area_key();
            off.iter().any(|a| a.key == key)
        });
        let mut text = format!(
            "Switch off {}?\n\nefsp will add Windows Firewall rules so eFootball can't reach \
             game servers there. Login and the store aren't affected. Peer-to-peer matches \
             start through one of the game's servers, so they may be affected too.\n\n\
             Experimental and not endorsed by Konami: matchmaking may take longer or fail. \
             Turn servers back on any time from the sidebar or the Servers page.",
            names.join(", ")
        );
        if usual_off && let Some(u) = &usual {
            text.push_str(&format!("\n\n{} is where you usually play.", u.label()));
        }
        text.push_str(
            "\n\nWindows will then ask for permission (check the taskbar if you don't see it).",
        );
        let mut dialog = app.dialog().message(text).title("Switch off servers");
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.parent(&window);
        }
        let confirmed = dialog
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Switch off".into(),
                "Cancel".into(),
            ))
            .blocking_show();
        if !confirmed {
            return Ok(None);
        }
        // A match may have started while the dialog was open.
        check_apply(&monitor.facts(), &off)?;

        let request = efsp_core::picker::HelperRequest::Apply {
            program,
            areas: off.clone(),
        };
        match picker::run_helper(&state.config_dir, &request)? {
            picker::HelperOutcome::Cancelled => Ok(None),
            picker::HelperOutcome::Done(_) => {
                saved.off = off;
                picker::save(&state.config_dir, &saved)?;
                Ok(Some(picker_status_now(&state, &monitor)))
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Removes every efsp rule (one UAC prompt). Always allowed, even mid-match.
fn remove_rules(app: &AppHandle) -> Result<Option<PickerStatus>, String> {
    let state = app.state::<AppState>();
    let monitor = app.state::<MonitorState>();
    match picker::run_helper(
        &state.config_dir,
        &efsp_core::picker::HelperRequest::RemoveAll,
    )? {
        picker::HelperOutcome::Cancelled => Ok(None),
        picker::HelperOutcome::Done(_) => {
            let mut saved = picker::load(&state.config_dir);
            saved.off.clear();
            picker::save(&state.config_dir, &saved)?;
            Ok(Some(picker_status_now(&state, &monitor)))
        }
    }
}

#[tauri::command]
async fn picker_remove_all(app: AppHandle) -> Result<Option<PickerStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(_busy) = PICKER_BUSY.try_lock() else {
            return Err("efsp is already changing the rules.".into());
        };
        remove_rules(&app)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Lets the player point efsp at eFootball.exe when Steam's records don't.
#[tauri::command]
async fn picker_choose_exe(app: AppHandle) -> Result<Option<PickerStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let monitor = app.state::<MonitorState>();
        let Some(chosen) = app
            .dialog()
            .file()
            .add_filter("eFootball", &["exe"])
            .set_title("Find eFootball.exe")
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = chosen.into_path().map_err(|e| e.to_string())?;
        if !picker::is_game_exe(&path) {
            return Err("That isn't eFootball.exe.".into());
        }
        let mut saved = picker::load(&state.config_dir);
        saved.game_exe = Some(path.to_string_lossy().into_owned());
        picker::save(&state.config_dir, &saved)?;
        Ok(Some(picker_status_now(&state, &monitor)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod picker_guard_tests {
    use super::*;
    use efsp_core::picker::Area;
    use monitor::LiveFacts;

    fn area(key: &str) -> Area {
        Area {
            key: key.into(),
            name: key.into(),
        }
    }

    fn facts(live: bool, match_active: bool, pinged: &[&str]) -> LiveFacts {
        LiveFacts {
            live,
            match_active,
            pinged_areas: pinged.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn apply_needs_live_capture_and_no_match() {
        let off = [area("a")];
        assert!(check_apply(&facts(false, false, &[]), &off).is_err());
        assert!(check_apply(&facts(true, true, &[]), &off).is_err());
        assert!(check_apply(&facts(true, false, &[]), &off).is_ok());
    }

    #[test]
    fn apply_keeps_a_pinged_city_on() {
        let f = facts(true, false, &["a", "b"]);
        assert!(check_apply(&f, &[area("a"), area("b")]).is_err());
        assert!(check_apply(&f, &[area("a")]).is_ok());
    }
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

/// Developer-tool commands. Compiled, registered and permitted only in
/// `dev-tools` builds (see build.rs and capabilities-dev/); a release build
/// has none of them.
#[cfg(feature = "dev-tools")]
mod dev_commands {
    use super::*;

    /// Fills history with 150 sample matches (dev builds only), so the
    /// History and Servers pages can be tried without playing. Returns how
    /// many were added.
    #[tauri::command]
    pub fn dev_seed_history(state: State<'_, AppState>) -> Result<usize, String> {
        let records = crate::sample::records(unix_now_s(), 150);
        history_of(&state)?.with(|s| {
            for r in &records {
                s.insert(r).map_err(|e| e.to_string())?;
            }
            Ok(records.len())
        })
    }

    /// Turns the header-only, anonymised trace recorder on or off.
    #[tauri::command]
    pub fn set_recording(state: State<'_, MonitorState>, on: bool) -> Result<(), String> {
        state.set_recording(on)
    }

    /// Opens the folder where recorded traces are saved.
    #[tauri::command]
    pub fn open_traces_folder(app: AppHandle) -> Result<(), String> {
        let dir = traces_dir(&app)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        app.opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    }

    /// Pings the current match target with Windows' own ping.exe for ~10 s
    /// and compares its median with ours. Returns numbers only.
    #[tauri::command]
    pub async fn compare_with_ping(
        state: State<'_, MonitorState>,
    ) -> Result<efsp_core::pingexe::Comparison, String> {
        let rx = state.compare_ping()?;
        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(30))
                .map_err(|_| "The ping check timed out.".to_owned())?
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Sends one TTL-1 probe and reports whether "TTL expired" came back.
    #[tauri::command]
    pub async fn test_route_probe(state: State<'_, MonitorState>) -> Result<String, String> {
        let rx = state.test_route_probe()?;
        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|_| "The test timed out.".to_owned())?
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Experiment: ICMP the nearest servers the game itself pings and
    /// compare. Returns numbered rows, no addresses.
    #[tauri::command]
    pub async fn compare_game_pings(
        state: State<'_, MonitorState>,
    ) -> Result<Vec<efsp_core::gameping::ServerComparison>, String> {
        let rx = state.compare_game_pings()?;
        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_secs(60))
                .map_err(|_| "The comparison timed out.".to_owned())?
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

fn traces_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("traces"))
        .map_err(|e| e.to_string())
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
        .manage(MonitorState::default())
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

#[cfg(not(feature = "dev-tools"))]
fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        npcap_status,
        open_npcap_download,
        start_monitor,
        history_list,
        history_detail,
        history_export_csv,
        history_delete_all,
        picker_status,
        picker_apply,
        picker_remove_all,
        picker_choose_exe,
        settings_get,
        settings_set
    ]
}

#[cfg(feature = "dev-tools")]
fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        npcap_status,
        open_npcap_download,
        start_monitor,
        history_list,
        history_detail,
        history_export_csv,
        history_delete_all,
        picker_status,
        picker_apply,
        picker_remove_all,
        picker_choose_exe,
        settings_get,
        settings_set,
        dev_commands::dev_seed_history,
        dev_commands::set_recording,
        dev_commands::open_traces_folder,
        dev_commands::compare_with_ping,
        dev_commands::compare_game_pings,
        dev_commands::test_route_probe
    ]
}
