//! Which local ports the game is using.
//!
//! The Windows implementation (app crate) reads the OS socket table, the same
//! data `netstat -ano` shows. It never opens or reads the game process itself.
//! This module holds the trait and the pure capture-filter builder.

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::capture::Protocol;

/// Executable name to look for (confirmed on Windows).
pub const GAME_PROCESS: &str = "eFootball.exe";

/// How often to re-read the game's ports.
pub const POLL_INTERVAL_MS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GamePort {
    pub protocol: Protocol,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameState {
    NotRunning,
    Running { ports: BTreeSet<GamePort> },
}

pub trait PortWatcher {
    /// Errors are messages for the UI (e.g. the socket table couldn't be
    /// read); they must not contain addresses.
    fn poll(&mut self) -> Result<GameState, String>;
}

/// Builds a BPF capture filter matching only the game's local ports, e.g.
/// `(udp port 50000) or (tcp port 51000)`. `None` when there is nothing to
/// capture, in which case capture should stay paused.
///
/// "port N" in BPF matches source or destination, which is what we want: the
/// game's port is the source going out and the destination coming in.
pub fn capture_filter(ports: &BTreeSet<GamePort>) -> Option<String> {
    let mut filter = String::new();
    for (i, gp) in ports.iter().filter(|gp| gp.port != 0).enumerate() {
        if i > 0 {
            filter.push_str(" or ");
        }
        let proto = match gp.protocol {
            Protocol::Udp => "udp",
            Protocol::Tcp => "tcp",
        };
        // Writing to a String can't fail.
        let _ = write!(filter, "({proto} port {})", gp.port);
    }
    (!filter.is_empty()).then_some(filter)
}

/// Replays a scripted list of states; repeats the last one. For tests and
/// for running the app on macOS.
#[derive(Debug)]
pub struct FakePortWatcher {
    script: Vec<GameState>,
    next: usize,
}

impl FakePortWatcher {
    pub fn new(script: Vec<GameState>) -> Self {
        Self { script, next: 0 }
    }
}

impl PortWatcher for FakePortWatcher {
    fn poll(&mut self) -> Result<GameState, String> {
        let state = self
            .script
            .get(self.next)
            .or(self.script.last())
            .cloned()
            .unwrap_or(GameState::NotRunning);
        self.next += 1;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(list: &[(Protocol, u16)]) -> BTreeSet<GamePort> {
        list.iter()
            .map(|&(protocol, port)| GamePort { protocol, port })
            .collect()
    }

    #[test]
    fn filter_lists_each_port_with_its_protocol() {
        let f = capture_filter(&ports(&[(Protocol::Tcp, 51000), (Protocol::Udp, 50000)]));
        assert_eq!(f.as_deref(), Some("(udp port 50000) or (tcp port 51000)"));
    }

    #[test]
    fn no_ports_means_no_filter() {
        assert_eq!(capture_filter(&BTreeSet::new()), None);
        assert_eq!(capture_filter(&ports(&[(Protocol::Udp, 0)])), None);
    }

    #[test]
    fn fake_watcher_follows_script_then_repeats_last() {
        let running = GameState::Running {
            ports: ports(&[(Protocol::Udp, 50000)]),
        };
        let mut w = FakePortWatcher::new(vec![GameState::NotRunning, running.clone()]);
        assert_eq!(w.poll(), Ok(GameState::NotRunning));
        assert_eq!(w.poll(), Ok(running.clone()));
        assert_eq!(w.poll(), Ok(running));
    }
}
