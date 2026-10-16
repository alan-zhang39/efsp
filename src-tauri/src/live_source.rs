//! Windows: live capture through Npcap, header-only and limited to the game's
//! ports.
//!
//! Safety properties, enforced here:
//! - Non-promiscuous, so only this PC's own traffic is seen.
//! - At most [`SNAPLEN`] bytes per packet (headers), then only header fields
//!   are parsed. No payload is kept.
//! - A BPF filter limits capture to the game's local ports. Packets are also
//!   re-checked in software, in case any slip in while the filter changes.
//! - The pcap handle is private and only read from. Nothing here can send,
//!   drop, delay or modify a packet.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::net::{IpAddr, UdpSocket};

use efsp_core::capture::{CaptureError, Next, PacketSource};
use efsp_core::parse::{LocalAddrs, SNAPLEN, parse_frame};
use efsp_core::ports::{GamePort, capture_filter};
use pcap::{Active, Capture, Device, Linktype};

/// How long one read waits before returning [`Next::Idle`].
const READ_TIMEOUT_MS: i32 = 100;

pub struct LiveSource {
    capture: Capture<Active>,
    local: LocalAddrs,
    ports: BTreeSet<GamePort>,
}

impl LiveSource {
    /// Opens the adapter that carries internet traffic.
    ///
    /// Only call this after `npcap_status` reported Npcap as found: wpcap.dll
    /// is delay-loaded, and calling into it when it's missing would crash.
    pub fn open() -> Result<Self, CaptureError> {
        npcap_dll::add_npcap_to_dll_search_path()?;

        let device = internet_device()?;
        let local = LocalAddrs::new(device.addresses.iter().map(|a| a.addr));
        let capture = Capture::from_device(device)
            .and_then(|c| {
                c.promisc(false)
                    .snaplen(SNAPLEN as i32)
                    .timeout(READ_TIMEOUT_MS)
                    .immediate_mode(true)
                    .open()
            })
            .map_err(device_err)?;

        if capture.get_datalink() != Linktype::ETHERNET {
            return Err(CaptureError::Device(
                "the network adapter isn't Ethernet-framed".into(),
            ));
        }

        let mut source = Self {
            capture,
            local,
            ports: BTreeSet::new(),
        };
        // Until the game's ports are known, match nothing at all.
        source.capture.filter("less 0", true).map_err(device_err)?;
        Ok(source)
    }

    /// Updates the capture filter when the game's ports change.
    pub fn set_ports(&mut self, ports: &BTreeSet<GamePort>) -> Result<(), CaptureError> {
        if *ports == self.ports {
            return Ok(());
        }
        let filter = capture_filter(ports).unwrap_or_else(|| "less 0".into());
        self.capture.filter(&filter, true).map_err(device_err)?;
        self.ports = ports.clone();
        Ok(())
    }
}

impl PacketSource for LiveSource {
    fn next(&mut self) -> Result<Next, CaptureError> {
        let packet = match self.capture.next_packet() {
            Ok(p) => p,
            Err(pcap::Error::TimeoutExpired) => return Ok(Next::Idle),
            Err(e) => return Err(device_err(e)),
        };
        // Stamped on receipt (immediate mode keeps this within a millisecond
        // or so of capture), so packets and the UI's "now" share one clock.
        // Npcap's own timestamps may drift from it.
        let header = parse_frame(packet.data, packet.header.len, unix_now_us(), &self.local);

        // Belt and braces: keep only packets on one of the game's ports.
        Ok(match header {
            Some(h)
                if self.ports.contains(&GamePort {
                    protocol: h.protocol,
                    port: h.local_port,
                }) =>
            {
                Next::Packet(h)
            }
            _ => Next::Idle,
        })
    }
}

fn unix_now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

fn device_err(e: pcap::Error) -> CaptureError {
    CaptureError::Device(e.to_string())
}

/// Finds the adapter Windows would use to reach the internet.
///
/// `connect` on a UDP socket only asks the OS for a route; it sends nothing.
/// 192.0.2.1 is a reserved documentation address, so no real host is involved.
fn internet_device() -> Result<Device, CaptureError> {
    let probe = UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|()| s))
        .and_then(|s| s.local_addr())
        .map_err(|e| CaptureError::Device(format!("no network route: {e}")))?;
    let local_ip: IpAddr = probe.ip();

    Device::list()
        .map_err(device_err)?
        .into_iter()
        .find(|d| d.addresses.iter().any(|a| a.addr == local_ip))
        .ok_or_else(|| CaptureError::Device("no capture adapter matches the internet route".into()))
}

mod npcap_dll {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use efsp_core::capture::CaptureError;
    use windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW;

    /// Npcap installs wpcap.dll in `System32\Npcap`, which Windows doesn't
    /// search. wpcap.dll is delay-loaded (see build.rs), so pointing the DLL
    /// search path there before the first pcap call is enough. Runs once; the
    /// outcome is remembered.
    pub fn add_npcap_to_dll_search_path() -> Result<(), CaptureError> {
        let dir = npcap_dir();
        // Checked on every open, not once: if Npcap was uninstalled since the
        // status check, the delay-loaded first pcap call would crash the app.
        if !dir.join("wpcap.dll").is_file() {
            return Err(CaptureError::Device("Npcap is no longer installed".into()));
        }
        static DONE: OnceLock<bool> = OnceLock::new();
        if *DONE.get_or_init(|| set_dll_directory(&dir)) {
            Ok(())
        } else {
            Err(CaptureError::Device(
                "couldn't add the Npcap folder to the DLL search path".into(),
            ))
        }
    }

    fn npcap_dir() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(root).join(r"System32\Npcap")
    }

    /// One of efsp's three audited `unsafe` spots.
    ///
    /// Note: this changes the DLL search path for the whole process, which is
    /// intended. It also removes the current directory from the search order,
    /// a small hardening bonus.
    #[allow(unsafe_code)]
    fn set_dll_directory(dir: &Path) -> bool {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the
        // call, which is all SetDllDirectoryW requires. It copies the path and
        // returns non-zero on success.
        unsafe { SetDllDirectoryW(wide.as_ptr()) != 0 }
    }
}
