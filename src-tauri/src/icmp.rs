//! Windows: sends one ICMP echo, optionally with a TTL, and reports what came
//! back. Uses `IcmpSendEcho` from iphlpapi.dll, the API `ping.exe` and
//! `tracert.exe` use; it needs no admin rights.
//!
//! Probes are tiny (32 bytes of zeros) and rate-limited by the cascade
//! (≤ 5/s), so they can't affect the match. Nothing here touches
//! game packets; these are separate packets of our own.

#![cfg(windows)]

use std::net::Ipv4Addr;

use efsp_core::cascade::ProbeResult;

/// How long to wait for a reply before counting the probe as lost.
pub const TIMEOUT_MS: u32 = 1000;
/// A `static`, not a `const`, so its address is fixed for the program's life
/// and the pointer handed to Windows is unquestionably valid during the call.
static PAYLOAD: [u8; 32] = [0; 32];

/// Sends one echo to `target`. With `ttl: Some(n)` it expires at the n-th
/// router, which answers "TTL expired" (that's how route estimates work).
/// Blocks for up to [`TIMEOUT_MS`]; run it off the monitor thread.
pub fn probe(target: Ipv4Addr, ttl: Option<u8>) -> ProbeResult {
    match ffi::send_echo(target, ttl) {
        Some(ffi::Reply::Echo { rtt_ms }) => ProbeResult::EchoReply {
            rtt_us: rtt_ms.saturating_mul(1000),
        },
        Some(ffi::Reply::TtlExpired { rtt_ms }) => ProbeResult::TtlExpired {
            rtt_us: rtt_ms.saturating_mul(1000),
        },
        None => ProbeResult::Lost,
    }
}

/// The third audited `unsafe` module in efsp .
mod ffi {
    use std::net::Ipv4Addr;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ICMP_ECHO_REPLY, IP_OPTION_INFORMATION, IP_SUCCESS, IP_TTL_EXPIRED_TRANSIT,
        IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
    };

    use super::{PAYLOAD, TIMEOUT_MS};

    pub enum Reply {
        Echo { rtt_ms: u32 },
        TtlExpired { rtt_ms: u32 },
    }

    /// Closes the ICMP handle however we leave `send_echo`.
    struct IcmpHandle(HANDLE);

    impl Drop for IcmpHandle {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: `self.0` came from IcmpCreateFile, was checked against
            // INVALID_HANDLE_VALUE, and is closed exactly once here.
            unsafe { IcmpCloseHandle(self.0) };
        }
    }

    #[allow(unsafe_code)]
    pub fn send_echo(target: Ipv4Addr, ttl: Option<u8>) -> Option<Reply> {
        // SAFETY: no arguments; failure is INVALID_HANDLE_VALUE, checked below.
        let raw = unsafe { IcmpCreateFile() };
        if raw == INVALID_HANDLE_VALUE {
            return None;
        }
        let handle = IcmpHandle(raw);

        let options = IP_OPTION_INFORMATION {
            Ttl: ttl.unwrap_or(128),
            ..Default::default()
        };
        // Room for one reply struct plus the echoed payload and an ICMP error
        // body (Microsoft's guidance: struct + data + 8, plus slack). Using a
        // Vec of the struct type keeps the buffer correctly aligned.
        let mut buffer = vec![ICMP_ECHO_REPLY::default(); 4];
        let buffer_bytes = (buffer.len() * size_of::<ICMP_ECHO_REPLY>()) as u32;

        // SAFETY:
        // - `handle.0` is a valid ICMP handle (checked above).
        // - The destination is an IPv4 address in network byte order, which is
        //   the in-memory order of `octets()`.
        // - PAYLOAD and `options` live until the call returns.
        // - `buffer` is writable, aligned for ICMP_ECHO_REPLY and
        //   `buffer_bytes` long; the call writes at most that much.
        // The call blocks until a reply or the timeout; nothing is kept after.
        let replies = unsafe {
            IcmpSendEcho(
                handle.0,
                u32::from_ne_bytes(target.octets()),
                PAYLOAD.as_ptr().cast(),
                PAYLOAD.len() as u16,
                &options,
                buffer.as_mut_ptr().cast(),
                buffer_bytes,
                TIMEOUT_MS,
            )
        };
        if replies == 0 {
            return None; // timed out or unreachable
        }
        let reply = &buffer[0];
        match reply.Status {
            // Only count an echo reply if it came from the target itself.
            IP_SUCCESS if reply.Address == u32::from_ne_bytes(target.octets()) => {
                Some(Reply::Echo {
                    rtt_ms: reply.RoundTripTime,
                })
            }
            IP_TTL_EXPIRED_TRANSIT => Some(Reply::TtlExpired {
                rtt_ms: reply.RoundTripTime,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real Windows call against this machine's own loopback,
    /// which always answers. Runs in CI on Windows.
    #[test]
    fn loopback_answers() {
        match probe(Ipv4Addr::LOCALHOST, None) {
            ProbeResult::EchoReply { rtt_us } => assert!(rtt_us < 100_000, "{rtt_us}"),
            other => panic!("expected an echo reply from 127.0.0.1, got {other:?}"),
        }
    }

    /// TTL 1 expires at the first router, which should answer "TTL expired".
    /// CI networks may not send that reply, so this only rejects nonsense and
    /// logs the outcome (visible in the CI log); route estimates are
    /// checked by hand on a real network.
    #[test]
    fn ttl_one_never_reaches_the_target() {
        let r = probe(Ipv4Addr::new(192, 0, 2, 1), Some(1));
        println!("TTL 1 probe result on this network: {r:?}");
        assert!(!matches!(r, ProbeResult::EchoReply { .. }), "{r:?}");
    }

    /// A documentation address never answers: the probe must come back lost,
    /// within the timeout, not hang or crash.
    #[test]
    fn unreachable_target_is_lost() {
        let started = std::time::Instant::now();
        assert_eq!(probe(Ipv4Addr::new(192, 0, 2, 1), None), ProbeResult::Lost);
        assert!(started.elapsed().as_millis() < u128::from(TIMEOUT_MS) + 1500);
    }
}
