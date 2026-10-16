//! Windows: finds eFootball.exe and the local ports it has open.
//!
//! Reads two OS tables and nothing else:
//! - the process list, via a Toolhelp snapshot (names and PIDs), and
//! - the socket table (`GetExtendedUdpTable`/`GetExtendedTcpTable`, via
//!   `netstat2`), the same data `netstat -ano` shows.
//!
//! Neither opens a handle to any process, so the game is never touched.
//! (The `sysinfo` crate was dropped for this reason: it opens every
//! process with memory-read rights.)

#![cfg(windows)]

use std::collections::BTreeSet;

use efsp_core::capture::Protocol;
use efsp_core::ports::{GAME_PROCESS, GamePort, GameState, PortWatcher};
use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, get_sockets_info};

#[derive(Default)]
pub struct WindowsPortWatcher;

impl PortWatcher for WindowsPortWatcher {
    fn poll(&mut self) -> Result<GameState, String> {
        let pids = toolhelp::pids_named(GAME_PROCESS)
            .map_err(|e| format!("couldn't list processes: {e}"))?;
        if pids.is_empty() {
            return Ok(GameState::NotRunning);
        }
        let sockets = get_sockets_info(
            AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
            ProtocolFlags::TCP | ProtocolFlags::UDP,
        )
        .map_err(|e| format!("couldn't read the socket table: {e}"))?;

        let ports: BTreeSet<GamePort> = sockets
            .into_iter()
            .filter(|s| s.associated_pids.iter().any(|pid| pids.contains(pid)))
            .map(|s| match s.protocol_socket_info {
                // Only the local port is used; the remote side of TCP sockets
                // is ignored here (flows learn remotes from captured headers).
                ProtocolSocketInfo::Tcp(tcp) => GamePort {
                    protocol: Protocol::Tcp,
                    port: tcp.local_port,
                },
                ProtocolSocketInfo::Udp(udp) => GamePort {
                    protocol: Protocol::Udp,
                    port: udp.local_port,
                },
            })
            .collect();
        Ok(GameState::Running { ports })
    }
}

/// The second audited `unsafe` module in efsp .
mod toolhelp {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    /// Closes the snapshot handle however we leave `pids_named`.
    struct Snapshot(HANDLE);

    impl Drop for Snapshot {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: `self.0` is a valid snapshot handle we own (checked
            // against INVALID_HANDLE_VALUE on creation) and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// PIDs of every process whose executable name matches `name`
    /// (case-insensitive). Reads the system's process list; opens no process.
    #[allow(unsafe_code)]
    pub fn pids_named(name: &str) -> io::Result<Vec<u32>> {
        // SAFETY: plain FFI call with constant arguments; failure is reported
        // as INVALID_HANDLE_VALUE, which we check.
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Snapshot(handle);

        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut pids = Vec::new();
        // SAFETY: `snapshot.0` is a valid process snapshot and `entry` is a
        // live, correctly sized PROCESSENTRY32W (dwSize set) that the API
        // writes into. Nothing is retained after each call returns.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if exe.eq_ignore_ascii_case(name) {
                pids.push(entry.th32ProcessID);
            }
            // SAFETY: as for Process32FirstW above.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        Ok(pids)
    }
}
