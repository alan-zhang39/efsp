//! Region picker, app side. The rules themselves come
//! from `efsp_core::picker`; this module finds eFootball.exe, remembers which
//! cities are switched off, and runs the elevated helper.
//!
//! The game's path comes from Steam's own install records (or a file the
//! player picks), never from the running game: opening the game's process
//! is off limits.

use std::path::{Path, PathBuf};

use efsp_core::firewall::GAME_EXE;
use efsp_core::picker::{Area, HelperRequest};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "picker.json";
const REQUEST_FILE: &str = "picker-request.json";
const RESULT_FILE: &str = "picker-result.json";
pub const HELPER_EXE: &str = "efsp-helper.exe";

/// What efsp last applied, kept in its config folder. Rules stay on while
/// efsp is closed, so this is how the app knows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PickerState {
    /// Cities switched off; empty = no rules.
    pub off: Vec<Area>,
    /// The eFootball.exe rules are scoped to, once found or chosen.
    pub game_exe: Option<String>,
}

pub fn load(config_dir: &Path) -> PickerState {
    std::fs::read(config_dir.join(STATE_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, state: &PickerState) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let tmp = config_dir.join(format!("{STATE_FILE}.tmp"));
    let json = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, config_dir.join(STATE_FILE)).map_err(|e| e.to_string())
}

/// True if `path` names eFootball.exe (any case) and exists.
pub fn is_game_exe(path: &Path) -> bool {
    path.file_name()
        .and_then(|f| f.to_str())
        .is_some_and(|f| f.eq_ignore_ascii_case(GAME_EXE))
        && path.is_file()
}

/// eFootball's Steam app id: Steam records each installed game in
/// `steamapps/appmanifest_<id>.acf`, including the folder it's in.
pub const EFOOTBALL_STEAM_APP: &str = "1665460";

/// The value of a top-level `"key"  "value"` line in a Steam `.vdf`/`.acf`
/// file (backslashes are escaped as `\\` there).
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows Steam lookup
fn vdf_value(text: &str, key: &str) -> Option<String> {
    let quoted = format!("\"{key}\"");
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(&quoted)?.trim();
        let value = rest.strip_prefix('"')?.strip_suffix('"')?;
        Some(value.replace("\\\\", "\\"))
    })
}

/// Library folders listed in Steam's `libraryfolders.vdf` (`"path" "…"`
/// lines; backslashes are escaped as `\\` in that file).
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows Steam lookup
pub fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("\"path\"")?.trim();
            let value = rest.strip_prefix('"')?.strip_suffix('"')?;
            Some(PathBuf::from(value.replace("\\\\", "\\")))
        })
        .collect()
}

/// eFootball.exe in any of these Steam libraries, if installed.
#[cfg_attr(not(windows), allow(dead_code))] // used by the Windows Steam lookup
pub fn find_in_libraries(libraries: &[PathBuf]) -> Option<PathBuf> {
    // Steam's own record of where it installed eFootball, in any library.
    for lib in libraries {
        let steamapps = lib.join("steamapps");
        let manifest = steamapps.join(format!("appmanifest_{EFOOTBALL_STEAM_APP}.acf"));
        if let Ok(acf) = std::fs::read_to_string(&manifest)
            && let Some(dir) = vdf_value(&acf, "installdir")
        {
            let exe = steamapps.join("common").join(dir).join(GAME_EXE);
            if is_game_exe(&exe) {
                return Some(exe);
            }
        }
    }
    // No record: the usual folder, then any game folder holding the exe.
    for lib in libraries {
        let common = lib.join("steamapps").join("common");
        // The usual folder first, then any game folder holding the exe.
        let usual = common.join("eFootball").join(GAME_EXE);
        if is_game_exe(&usual) {
            return Some(usual);
        }
        if let Ok(dirs) = std::fs::read_dir(&common) {
            for dir in dirs.flatten() {
                let exe = dir.path().join(GAME_EXE);
                if is_game_exe(&exe) {
                    return Some(exe);
                }
            }
        }
    }
    None
}

/// Finds eFootball.exe through Steam's install records: Steam itself from
/// the user's and the machine's registry (or its default folder), then
/// every library it lists.
#[cfg(windows)]
pub fn find_game_exe() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let read = |hive, key: &str, value: &str| -> Option<String> {
        RegKey::predef(hive)
            .open_subkey(key)
            .ok()?
            .get_value(value)
            .ok()
    };
    let mut roots: Vec<PathBuf> = [
        read(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        read(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
        read(HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
        Some(r"C:\Program Files (x86)\Steam".to_owned()),
    ]
    .into_iter()
    .flatten()
    .map(|p| PathBuf::from(p.replace('/', "\\")))
    .collect();
    roots.dedup_by(|a, b| {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    });
    let mut libraries = roots.clone();
    for root in &roots {
        // Newer Steam keeps the list in steamapps, older in config.
        for file in [
            root.join("steamapps").join("libraryfolders.vdf"),
            root.join("config").join("libraryfolders.vdf"),
        ] {
            if let Ok(vdf) = std::fs::read_to_string(file) {
                libraries.extend(library_paths(&vdf));
            }
        }
    }
    find_in_libraries(&libraries)
}

#[cfg(not(windows))]
pub fn find_game_exe() -> Option<PathBuf> {
    None
}

/// The helper installed next to efsp, if this build has one.
pub fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let helper = exe.parent()?.join(HELPER_EXE);
    helper.is_file().then_some(helper)
}

/// Whether any of efsp's rules exist right now, asked without admin rights
/// (netsh's exit code only, never its translated text). `None` if unknown.
#[cfg(windows)]
pub fn rules_present() -> Option<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let root: String = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value("SystemRoot"))
        .ok()?;
    let mut cmd = std::process::Command::new(Path::new(&root).join("System32").join("netsh.exe"));
    cmd.creation_flags(CREATE_NO_WINDOW);
    for a in efsp_core::firewall::netsh::show() {
        cmd.raw_arg(a);
    }
    cmd.output().ok().map(|o| o.status.success())
}

#[cfg(not(windows))]
pub fn rules_present() -> Option<bool> {
    None
}

#[derive(Debug, PartialEq, Eq)]
pub enum HelperOutcome {
    /// Rules changed; how many were added.
    Done(usize),
    /// The admin prompt was declined (or the helper never ran): nothing changed.
    Cancelled,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperResult {
    ok: bool,
    rules_added: usize,
    error: Option<String>,
}

/// Runs the helper with a UAC prompt and waits for it. Blocking.
pub fn run_helper(config_dir: &Path, request: &HelperRequest) -> Result<HelperOutcome, String> {
    let helper = helper_path().ok_or("The region picker isn't included in this build.")?;
    let req = config_dir.join(REQUEST_FILE);
    let res = config_dir.join(RESULT_FILE);
    let _ = std::fs::remove_file(&res);
    std::fs::write(
        &req,
        serde_json::to_vec(request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let started = elevate(&helper, &req, &res);
    let _ = std::fs::remove_file(&req);
    let Ok(bytes) = std::fs::read(&res) else {
        // Only a declined prompt counts as "nothing changed". Anything else
        // (a crash part-way, say) may have changed rules: the caller
        // re-reads the firewall's state.
        return match started {
            Elevation::Declined => Ok(HelperOutcome::Cancelled),
            Elevation::Ran(code) => Err(format!(
                "The firewall helper stopped unexpectedly (code {code}). efsp re-checked which rules are on."
            )),
            Elevation::Failed(e) => Err(e),
        };
    };
    let _ = std::fs::remove_file(&res);
    let result: HelperResult = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if result.ok {
        Ok(HelperOutcome::Done(result.rules_added))
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "The firewall change failed.".into()))
    }
}

#[cfg_attr(not(windows), allow(dead_code))] // only Windows elevates
enum Elevation {
    /// The UAC prompt was declined (or couldn't be shown): the helper never ran.
    Declined,
    /// The helper ran and exited with this code.
    Ran(i32),
    Failed(String),
}

#[cfg(windows)]
fn elevate(helper: &Path, req: &Path, res: &Path) -> Elevation {
    match runas::Command::new(helper)
        .arg(req)
        .arg(res)
        .gui(true)
        .show(false)
        .status()
    {
        // runas reports 0xFFFFFFFF only when ShellExecuteEx fails, which is
        // what a declined prompt looks like.
        Ok(status) if status.code() == Some(-1) => Elevation::Declined,
        Ok(status) => Elevation::Ran(status.code().unwrap_or(-2)),
        Err(e) => Elevation::Failed(e.to_string()),
    }
}

#[cfg(not(windows))]
fn elevate(_helper: &Path, _req: &Path, _res: &Path) -> Elevation {
    Elevation::Failed("Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steam_library_folders() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "1665460" "51234567890" }
	}
}"#;
        assert_eq!(
            library_paths(vdf),
            [
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary")
            ]
        );
    }

    #[test]
    fn finds_the_game_in_a_library_and_checks_the_name() {
        let lib = std::env::temp_dir().join(format!("efsp-lib-{}", std::process::id()));
        let game = lib.join("steamapps").join("common").join("eFootball Game");
        std::fs::create_dir_all(&game).unwrap();
        assert_eq!(find_in_libraries(std::slice::from_ref(&lib)), None);
        std::fs::write(game.join(GAME_EXE), b"").unwrap();
        assert_eq!(
            find_in_libraries(std::slice::from_ref(&lib)),
            Some(game.join(GAME_EXE))
        );
        assert!(!is_game_exe(&game.join("other.exe")));
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn uses_steams_record_of_the_install_folder() {
        let lib = std::env::temp_dir().join(format!("efsp-acf-{}", std::process::id()));
        let steamapps = lib.join("steamapps");
        // A folder name nothing would guess, as Steam allows.
        let game = steamapps.join("common").join("eFootball Live");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join(GAME_EXE), b"").unwrap();
        std::fs::write(
            steamapps.join(format!("appmanifest_{EFOOTBALL_STEAM_APP}.acf")),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"1665460\"\n\t\"installdir\"\t\t\"eFootball Live\"\n}\n",
        )
        .unwrap();
        assert_eq!(
            find_in_libraries(std::slice::from_ref(&lib)),
            Some(game.join(GAME_EXE))
        );
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn reads_values_from_steam_files() {
        let acf = "\"AppState\"\n{\n\t\"installdir\"\t\t\"eFootball\"\n}";
        assert_eq!(vdf_value(acf, "installdir").as_deref(), Some("eFootball"));
        assert_eq!(vdf_value(acf, "missing"), None);
    }

    #[test]
    fn state_round_trips_and_tolerates_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("efsp-picker-{}", std::process::id()));
        assert_eq!(load(&dir), PickerState::default());
        let state = PickerState {
            off: vec![Area {
                key: "server:Frankfurt|DE".into(),
                name: "Frankfurt".into(),
            }],
            game_exe: Some(r"C:\G\eFootball.exe".into()),
        };
        save(&dir, &state).unwrap();
        assert_eq!(load(&dir), state);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
