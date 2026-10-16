//! efsp's elevated helper. efsp starts it with a Windows UAC
//! prompt for each change; it does exactly one of two things:
//!
//! - **apply**: remove all of efsp's region-picker rules, then block the
//!   cities in the request (outbound UDP from eFootball.exe only);
//! - **removeAll**: remove all of efsp's region-picker rules.
//!
//! It trusts nothing in the request. The request names cities, not
//! addresses: the helper builds every rule itself from its own built-in
//! range table (`efsp_core::picker`), with the fixed `efsp: region picker`
//! name, and netsh's direction, action and protocol are fixed in code. If
//! adding any rule fails, it removes everything it added.
//!
//! Usage: `efsp-helper <request.json> <result.json>`. Exit code 0 = done;
//! the result file (created fresh, never overwritten) says what happened.

// No console window when efsp launches it.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::path::Path;
use std::process::ExitCode;

use efsp_core::firewall::{Rule, netsh};
use efsp_core::picker::HelperRequest;
use serde::Serialize;

/// Requests are a few KB.
const MAX_REQUEST_BYTES: u64 = 256 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Outcome {
    ok: bool,
    rules_added: usize,
    error: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [request, result] = args.as_slice() else {
        return ExitCode::from(2);
    };
    let outcome = match run(Path::new(request)) {
        Ok(n) => Outcome {
            ok: true,
            rules_added: n,
            error: None,
        },
        Err(e) => Outcome {
            ok: false,
            rules_added: 0,
            error: Some(e),
        },
    };
    let ok = outcome.ok;
    write_result(Path::new(result), &outcome);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Writes the outcome to a file that must not exist yet. Running as admin,
/// the helper never overwrites or follows an existing file at a path the
/// user's account controls (efsp deletes the old result first).
fn write_result(path: &Path, outcome: &Outcome) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        let _ = f.write_all(&serde_json::to_vec(outcome).unwrap_or_default());
    }
}

/// Reads and checks a request: the rules to add (none for removeAll).
fn read_request(path: &Path) -> Result<(HelperRequest, Vec<Rule>), String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("request too large".into());
    }
    let request: HelperRequest =
        serde_json::from_slice(&bytes).map_err(|e| format!("bad request: {e}"))?;
    let rules = request.rules().map_err(|e| e.to_string())?;
    Ok((request, rules))
}

fn run(path: &Path) -> Result<usize, String> {
    let (request, rules) = read_request(path)?;
    if let HelperRequest::Apply { program, .. } = &request
        && !Path::new(program).is_file()
    {
        return Err(format!("{program} doesn't exist"));
    }
    // Start clean. netsh fails when there's nothing to delete; that's fine,
    // but none of our rules may be left before adding new ones.
    let _ = run_netsh(&netsh::delete_all());
    if run_netsh(&netsh::show()).is_ok() {
        return Err("couldn't remove efsp's existing rules".into());
    }
    for (i, rule) in rules.iter().enumerate() {
        if let Err(e) = run_netsh(&netsh::add(rule)) {
            // All or nothing: don't leave half a selection in place.
            let _ = run_netsh(&netsh::delete_all());
            return Err(format!("rule {} of {}: {e}", i + 1, rules.len()));
        }
    }
    Ok(rules.len())
}

/// Windows' folder, from the machine-wide registry (only admins can change
/// it), never from environment variables the user's account can set.
#[cfg(windows)]
fn system_root() -> Result<std::path::PathBuf, String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let root: String = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value("SystemRoot"))
        .map_err(|e| format!("can't find Windows' folder: {e}"))?;
    Ok(root.into())
}

/// Runs Windows' own netsh.exe with these exact arguments: no shell, and no
/// re-quoting (each argument is already netsh-ready).
#[cfg(windows)]
fn run_netsh(args: &[String]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = system_root()?.join("System32").join("netsh.exe");
    let mut cmd = std::process::Command::new(exe);
    cmd.creation_flags(CREATE_NO_WINDOW);
    for a in args {
        cmd.raw_arg(a);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}

#[cfg(not(windows))]
fn run_netsh(_args: &[String]) -> Result<(), String> {
    Err("efsp-helper only runs on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("efsp-helper-{}-{name}", std::process::id()));
        std::fs::write(&p, body).unwrap();
        p
    }

    // Validation only: tests never run netsh.

    #[test]
    fn remove_all_needs_no_rules() {
        let (req, rules) = read_request(&write("remove", r#"{"op":"removeAll"}"#)).unwrap();
        assert_eq!(req, HelperRequest::RemoveAll);
        assert!(rules.is_empty());
    }

    #[test]
    fn apply_builds_the_rules_from_city_names() {
        let body = r#"{"op":"apply","program":"C:\\Games\\eFootball\\eFootball.exe",
            "areas":[{"key":"server:Frankfurt|DE","name":"Frankfurt"}]}"#;
        let (_, rules) = read_request(&write("apply", body)).unwrap();
        assert!(!rules.is_empty());
        assert!(
            rules
                .iter()
                .all(|r| r.name.as_str() == "efsp: region picker")
        );
    }

    #[test]
    fn never_overwrites_an_existing_result_file() {
        let p = write("existing", "keep me");
        write_result(
            &p,
            &Outcome {
                ok: true,
                rules_added: 1,
                error: None,
            },
        );
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "keep me");
    }

    #[test]
    fn hostile_requests_are_refused() {
        let big = format!(r#"{{"op":"removeAll","pad":"{}"}}"#, "x".repeat(300_000));
        for (name, body) in [
            ("junk", "not json"),
            ("op", r#"{"op":"deleteEverything"}"#),
            (
                "prog",
                r#"{"op":"apply","program":"C:\\Windows\\System32\\svchost.exe","areas":[{"key":"server:Frankfurt|DE","name":"Frankfurt"}]}"#,
            ),
            (
                "city",
                r#"{"op":"apply","program":"C:\\G\\eFootball.exe","areas":[{"key":"server:Nowhere|XX","name":"x"}]}"#,
            ),
            ("big", big.as_str()),
        ] {
            assert!(read_request(&write(name, body)).is_err(), "{name}");
        }
    }
}
