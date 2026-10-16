//! Runs the capture pipeline on a background thread and streams snapshots to
//! the UI.
//!
//! ```text
//! live (Windows):  port watcher ─► LiveSource ─┐
//! replay (dev):    ReplaySource ───────────────┴─► FlowTracker ─► Snapshot ─► UI
//!                                                 ├─► Session (probes, numbers)
//!                                                 └─► TraceWriter (dev builds, live only)
//! ```
//!
//! Snapshots go through a Tauri `Channel`, which the UI passes in when it calls
//! `start_monitor`. Channels need no extra window permissions.
//!
//! Developer tools (trace recorder, `EFSP_REPLAY`, the ping.exe cross-check
//! and the game-ping experiment) are compiled only with the `dev-tools`
//! feature. The release installer contains none of that code.

// A macOS build without dev-tools has no capture source at all, so the shared
// pipeline goes unused there. Every other build uses it.
#![cfg_attr(
    not(any(windows, feature = "dev-tools")),
    allow(dead_code, reason = "no capture source on this build")
)]

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use efsp_core::capture::{PacketHeader, Protocol};
use efsp_core::flow::{Candidate, FlowTracker, FlowView};
#[cfg(feature = "dev-tools")]
use efsp_core::gameping::ServerComparison;
use efsp_core::gameping::{GamePings, GamePingsView};
use efsp_core::session::{MeasurementView, Session};

use crate::geo::Geo;
use crate::store::History;
use serde::Serialize;
use tauri::ipc::Channel;

/// How often the UI gets a snapshot (wall-clock time).
#[cfg(feature = "dev-tools")]
const EMIT_EVERY: std::time::Duration = std::time::Duration::from_millis(250);
const EMIT_EVERY_US: u64 = 250_000;

/// A finished session only becomes the "last match" if it lasted this long.
/// Real match offers, even declined ones, last ~8–11 s (real traces); a
/// half-second burst can briefly look sustained but never lasts this long.
const MIN_LAST_MATCH_MS: u64 = 3_000;
/// Before a peer-to-peer match the game briefly talks to one of its servers
/// (5–7 s in real match history), which looks like a declined
/// offer. A short server session is held this long: if a peer match starts
/// meanwhile, it was that setup and isn't saved.
const PEER_SETUP_WINDOW_US: u64 = 30_000_000;

/// Whether this build includes the developer tools.
pub const DEV_TOOLS: bool = cfg!(feature = "dev-tools");

// Variants mirror the UI's types; which ones a given build constructs depends
// on the OS and on `dev-tools`, so unused ones are expected.
#[allow(dead_code, reason = "constructed per OS / dev-tools build")]
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Live,
    Replay,
    Unavailable,
}

#[allow(dead_code, reason = "constructed per OS / dev-tools build")]
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GameStatus {
    NotRunning,
    Running,
    ReplayEnded,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub file_name: String,
    pub packets: u64,
}

/// Everything the monitor screen shows. Contains no IP addresses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub source: SourceKind,
    pub game: GameStatus,
    pub game_ports: usize,
    /// Every flow the game has open, for the connections table. Empty in
    /// release builds, which don't show it.
    pub flows: Vec<FlowView>,
    /// Always `None` in release builds (no recorder).
    pub recording: Option<Recording>,
    /// Numbers for the match connection, if one is being measured.
    pub measurement: Option<MeasurementView>,
    /// Final numbers of the previous match, kept until the next one starts.
    pub last_match: Option<MeasurementView>,
    /// The game's own background pings, by region: your ping to each
    /// region for the Servers page, measured passively.
    pub game_pings: Option<GamePingsView>,
    /// Matches and offers saved to history this run, so pages showing
    /// history know when to reload.
    pub saved: u32,
    /// Possible TCP matches seen this run. Not in release builds.
    #[cfg(feature = "dev-tools")]
    pub tcp_matches: u32,
    pub dev_tools: bool,
    pub error: Option<String>,
}

impl Snapshot {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            source: SourceKind::Unavailable,
            game: GameStatus::NotRunning,
            game_ports: 0,
            flows: Vec::new(),
            recording: None,
            measurement: None,
            last_match: None,
            game_pings: None,
            saved: 0,
            #[cfg(feature = "dev-tools")]
            tcp_matches: 0,
            dev_tools: DEV_TOOLS,
            error: Some(reason.into()),
        }
    }
}

/// Requests from the UI to the pipeline thread. All are developer tools, so
/// a release build has an empty enum: nothing can be requested.
enum Control {
    #[cfg(feature = "dev-tools")]
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "recording is part of live capture, Windows-only")
    )]
    SetRecording(bool),
    /// Run `ping.exe` against the current target and compare.
    #[cfg(feature = "dev-tools")]
    ComparePing(Sender<Result<efsp_core::pingexe::Comparison, String>>),
    /// Experiment: ICMP the nearest game-pinged servers and compare.
    #[cfg(feature = "dev-tools")]
    CompareGamePings(Sender<Result<Vec<ServerComparison>, String>>),
    /// One TTL-1 probe, to confirm "TTL expired" replies are understood.
    #[cfg(feature = "dev-tools")]
    TestRouteProbe(Sender<Result<String, String>>),
}

type SharedChannel = Arc<Mutex<Option<Channel<Snapshot>>>>;
/// Tauri-managed state: the UI's current channel, and a way to talk to the
/// pipeline thread once it is running.
#[derive(Default)]
pub struct MonitorState {
    channel: SharedChannel,
    control: Mutex<Option<Sender<Control>>>,
    facts: Arc<Mutex<LiveFacts>>,
}

/// What the region picker needs to know from the latest snapshot.
#[derive(Debug, Clone, Default)]
pub struct LiveFacts {
    /// Live capture is running (not unavailable, not a replay): only then
    /// do the other facts mean anything.
    pub live: bool,
    /// A match connection is being measured: rules mustn't change now.
    pub match_active: bool,
    /// Cities the game is pinging right now
    /// ([`efsp_core::classify::Endpoint::area_key`]).
    pub pinged_areas: Vec<String>,
}

impl LiveFacts {
    fn from_snapshot(s: &Snapshot) -> Self {
        Self {
            live: s.source == SourceKind::Live,
            match_active: s.measurement.is_some(),
            pinged_areas: s
                .game_pings
                .iter()
                .flat_map(|g| &g.regions)
                .filter(|r| matches!(r.endpoint, efsp_core::classify::Endpoint::Server { .. }))
                .map(|r| r.endpoint.area_key())
                .collect(),
        }
    }
}

impl MonitorState {
    /// Starts the pipeline once; later calls (e.g. a page reload) just swap
    /// in the new channel.
    pub fn start(
        &self,
        on_update: Channel<Snapshot>,
        traces_dir: PathBuf,
        history: Option<Arc<History>>,
        geo: Geo,
    ) {
        *lock(&self.channel) = Some(on_update);
        let mut control = lock(&self.control);
        if control.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        *control = Some(tx);
        let out = Arc::clone(&self.channel);
        let facts = Arc::clone(&self.facts);
        std::thread::Builder::new()
            .name("efsp-monitor".into())
            .spawn(move || run(&rx, &out, &facts, &traces_dir, history, geo))
            .expect("failed to start the monitor thread");
    }

    /// The latest live facts for the region picker.
    pub fn facts(&self) -> LiveFacts {
        lock(&self.facts).clone()
    }

    #[cfg(feature = "dev-tools")]
    fn send(&self, c: Control) -> Result<(), String> {
        lock(&self.control)
            .as_ref()
            .ok_or("the monitor isn't running")?
            .send(c)
            .map_err(|_| "the monitor has stopped".to_owned())
    }

    #[cfg(feature = "dev-tools")]
    pub fn set_recording(&self, on: bool) -> Result<(), String> {
        self.send(Control::SetRecording(on))
    }

    /// Starts a `ping.exe` cross-check; the receiver gets the result in ~10 s.
    #[cfg(feature = "dev-tools")]
    pub fn compare_ping(
        &self,
    ) -> Result<Receiver<Result<efsp_core::pingexe::Comparison, String>>, String> {
        let (tx, rx) = mpsc::channel();
        self.send(Control::ComparePing(tx))?;
        Ok(rx)
    }

    /// Sends one TTL-1 probe; the receiver gets a plain-language result.
    #[cfg(feature = "dev-tools")]
    pub fn test_route_probe(&self) -> Result<Receiver<Result<String, String>>, String> {
        let (tx, rx) = mpsc::channel();
        self.send(Control::TestRouteProbe(tx))?;
        Ok(rx)
    }

    /// Starts the game-ping experiment; the receiver gets rows in ~5–25 s.
    #[cfg(feature = "dev-tools")]
    pub fn compare_game_pings(
        &self,
    ) -> Result<Receiver<Result<Vec<ServerComparison>, String>>, String> {
        let (tx, rx) = mpsc::channel();
        self.send(Control::CompareGamePings(tx))?;
        Ok(rx)
    }
}

fn unix_now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A poisoned mutex only means another thread panicked mid-update; the data
/// here (an optional channel/sender) is still usable.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn run(
    control: &Receiver<Control>,
    out: &SharedChannel,
    facts: &Mutex<LiveFacts>,
    traces_dir: &std::path::Path,
    history: Option<Arc<History>>,
    geo: Geo,
) {
    let emit = |s: Snapshot| {
        *lock(facts) = LiveFacts::from_snapshot(&s);
        if let Some(ch) = lock(out).as_ref() {
            // The page may be reloading; the next snapshot will get through.
            let _ = ch.send(s);
        }
    };

    #[cfg(feature = "dev-tools")]
    if let Some(path) = std::env::var_os("EFSP_REPLAY") {
        // Replays are never saved to the real history, and their addresses
        // are anonymised, so there's nothing to classify.
        let _ = (&history, &geo);
        let speed = std::env::var("EFSP_REPLAY_SPEED")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| *s > 0.0)
            .unwrap_or(1.0);
        let clock = replay::WallClock::new(speed);
        replay::run_replay(std::path::Path::new(&path), &clock, control, &emit);
        return;
    }

    #[cfg(windows)]
    {
        // wpcap.dll is delay-loaded: never touch it unless Npcap is usable.
        match crate::npcap_probe::status() {
            efsp_core::npcap::NpcapStatus::Found { .. } => {
                live::run_live(control, &emit, traces_dir, history, geo);
            }
            _ => emit(Snapshot::unavailable(
                "Npcap isn't ready, so live capture is off.",
            )),
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (control, traces_dir, history, geo);
        emit(Snapshot::unavailable(if DEV_TOOLS {
            "Live capture runs on Windows. On this machine, start the app with \
             EFSP_REPLAY=<trace file> to replay a recorded match."
        } else {
            "Live capture runs on Windows."
        }));
    }
}

/// Shared by live and replay: tracker, session, emit timer, and in dev builds
/// the recorder and the game-ping experiment. Time is passed in, in whatever
/// clock the caller uses (wall or trace).
struct Pipeline {
    tracker: FlowTracker,
    last_emit_us: Option<u64>,
    /// Measurement of the current match connection.
    session: Option<Session>,
    /// Bumped per new session, so late probe results for an old one are dropped.
    session_gen: u32,
    /// The previous match's final numbers, kept until the next one starts.
    last_match: Option<MeasurementView>,
    /// Saved match history (live capture only; never during replay).
    history: Option<Arc<History>>,
    /// A short server session waiting to see whether a peer match follows
    /// (see [`PEER_SETUP_WINDOW_US`]), with when it ended.
    pending_offer: Option<(efsp_core::history::NewRecord, u64)>,
    #[cfg(feature = "dev-tools")]
    recorder: Option<(
        efsp_core::trace::TraceWriter<std::io::BufWriter<std::fs::File>>,
        Recording,
    )>,
    game_pings: GamePings,
    #[cfg(feature = "dev-tools")]
    tcp_matches: u32,
    /// Records saved to history this run.
    saved: u32,
}

impl Pipeline {
    fn new(history: Option<Arc<History>>) -> Self {
        Self {
            tracker: FlowTracker::new(),
            last_emit_us: None,
            session: None,
            session_gen: 0,
            last_match: None,
            history,
            pending_offer: None,
            #[cfg(feature = "dev-tools")]
            recorder: None,
            game_pings: GamePings::new(),
            #[cfg(feature = "dev-tools")]
            tcp_matches: 0,
            saved: 0,
        }
    }

    /// Classify remotes with `geo` (country and network names).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn with_geo(mut self, geo: Geo) -> Self {
        self.tracker = FlowTracker::with_geo(geo);
        self
    }

    /// Picks what to measure. The current session keeps its connection while
    /// that stays active, so two busy flows can't keep stealing it from each
    /// other (each switch would restart the numbers). A new candidate takes
    /// over only once the current connection goes idle.
    /// `can_probe` says whether probes can be sent to a new candidate.
    fn track_session(&mut self, now_us: u64, can_probe: impl Fn(&Candidate) -> bool) {
        let candidate = self.tracker.match_candidate(now_us);
        if let Some(current) = &self.session
            && self.tracker.is_active(current.flow_id(), now_us)
        {
            // Keep the current connection, except that a UDP match always
            // takes over from a possible TCP one.
            let udp_over_tcp = current.protocol() == Protocol::Tcp
                && candidate
                    .as_ref()
                    .is_some_and(|c| c.protocol == Protocol::Udp);
            if !udp_over_tcp {
                return;
            }
        }
        if let Some(candidate) = candidate {
            self.end_session(now_us);
            let probe = can_probe(&candidate);
            let endpoint = self.tracker.endpoint_for(candidate.remote.get().ip());
            // A peer match right after a short server session: that session
            // was the peer match's setup, not an offer.
            if matches!(endpoint, efsp_core::classify::Endpoint::Peer { .. }) {
                self.pending_offer = None;
            } else {
                self.save_pending();
            }
            #[cfg(feature = "dev-tools")]
            if candidate.protocol == Protocol::Tcp {
                self.tcp_matches += 1;
            }
            let mut session = Session::new(candidate, probe, now_us);
            session.set_endpoint(endpoint);
            self.session = Some(session);
            self.session_gen += 1;
        }
    }

    /// Ends the current session (if any). If it was a real match (its flow
    /// was ever sustained and it lasted [`MIN_LAST_MATCH_MS`]), it becomes
    /// `last_match` and is saved to history; a short burst that was
    /// measured briefly is just dropped.
    /// The remote address goes with the dropped session either way.
    ///
    /// A match that goes silent for 3 s (the idle threshold) and resumes would
    /// be split in two. eFootball doesn't pause traffic at half-time, so
    /// this isn't handled.
    fn end_session(&mut self, now_us: u64) {
        if let Some(session) = self.session.take() {
            let finished = session.finish(now_us);
            if finished.sustained && finished.summary.duration_ms >= MIN_LAST_MATCH_MS {
                let record = efsp_core::history::NewRecord::from_finished(&finished, unix_now_s());
                let short_server = record.kind == efsp_core::history::Kind::Offer
                    && matches!(
                        record.endpoint,
                        Some(efsp_core::classify::Endpoint::Server { .. })
                    );
                self.save_pending();
                if short_server {
                    self.pending_offer = Some((record, now_us));
                } else {
                    self.save(&record);
                }
                self.last_match = Some(finished.summary);
            }
        }
    }

    fn save(&mut self, record: &efsp_core::history::NewRecord) {
        if let Some(h) = &self.history {
            // A failed save mustn't stop monitoring; the card still shows the
            // match. The message has no addresses.
            match h.with(|s| s.insert(record)) {
                Ok(_) => self.saved = self.saved.wrapping_add(1),
                Err(e) => eprintln!("efsp: couldn't save a match to history: {e}"),
            }
        }
    }

    /// Saves a held offer (no peer match followed it).
    fn save_pending(&mut self) {
        if let Some((record, _)) = self.pending_offer.take() {
            self.save(&record);
        }
    }

    fn observe(&mut self, h: &PacketHeader) -> Result<(), String> {
        self.tracker.observe(h);
        if let Some(session) = &mut self.session {
            session.observe(h);
        }
        self.game_pings.observe(h);
        #[cfg(feature = "dev-tools")]
        {
            if let Some((writer, info)) = &mut self.recorder {
                writer.write(h).map_err(|e| e.to_string())?;
                info.packets += 1;
            }
        }
        Ok(())
    }

    fn due(&self, now_us: u64, every_us: u64) -> bool {
        self.last_emit_us
            .is_none_or(|last| now_us.saturating_sub(last) >= every_us)
    }

    fn snapshot(
        &mut self,
        source: SourceKind,
        game: GameStatus,
        game_ports: usize,
        now_us: u64,
        error: Option<String>,
    ) -> Snapshot {
        self.last_emit_us = Some(now_us);
        if self
            .pending_offer
            .as_ref()
            .is_some_and(|(_, ended)| now_us.saturating_sub(*ended) >= PEER_SETUP_WINDOW_US)
        {
            self.save_pending();
        }
        self.tracker.tick(now_us);
        self.game_pings.tick(now_us);
        let status = self
            .session
            .as_ref()
            .and_then(|s| self.tracker.status(s.flow_id(), now_us));
        // The session ends when its connection goes idle or disappears.
        if self.session.is_some() && !status.is_some_and(|f| !f.idle) {
            self.end_session(now_us);
        }
        if let Some(session) = &mut self.session {
            if status.is_some_and(|f| f.sustained) {
                session.mark_sustained();
            }
            session.tick(now_us);
        }
        // Only developer builds show the connections table, so release
        // builds don't build or send it.
        let flows = if DEV_TOOLS {
            self.tracker.views(now_us)
        } else {
            Vec::new()
        };
        Snapshot {
            source,
            game,
            game_ports,
            measurement: self.session.as_mut().map(|s| s.view(now_us)),
            last_match: self.last_match.clone(),
            saved: self.saved,
            flows,
            #[cfg(feature = "dev-tools")]
            recording: self.recorder.as_ref().map(|(_, info)| info.clone()),
            #[cfg(not(feature = "dev-tools"))]
            recording: None,
            #[cfg(feature = "dev-tools")]
            tcp_matches: self.tcp_matches,
            game_pings: Some({
                let mut v = self.game_pings.view();
                let tracker = &mut self.tracker;
                // Same thresholds as the card's bars.
                v.regions = self.game_pings.regions(
                    &mut |ip| tracker.endpoint_for(ip),
                    &efsp_core::bars::BarThresholds::default(),
                );
                v
            }),
            dev_tools: DEV_TOOLS,
            error,
        }
    }
}

#[cfg(feature = "dev-tools")]
mod replay {
    use std::fs::File;
    use std::io::BufReader;
    use std::path::Path;
    use std::sync::mpsc::TryRecvError;
    use std::time::{Duration, Instant};

    use efsp_core::capture::{Next, PacketSource};
    use efsp_core::trace::ReplaySource;

    use super::*;

    /// Time source for replay, so tests can run without real sleeping.
    pub trait Clock {
        /// Trace time that corresponds to "now".
        fn now_us(&self) -> u64;
        fn speed(&self) -> f64;
        /// Waits `d` of wall-clock time.
        fn sleep(&self, d: Duration);
    }

    pub struct WallClock {
        started: Instant,
        speed: f64,
    }

    impl WallClock {
        pub fn new(speed: f64) -> Self {
            Self {
                started: Instant::now(),
                speed,
            }
        }
    }

    impl Clock for WallClock {
        fn now_us(&self) -> u64 {
            (self.started.elapsed().as_secs_f64() * self.speed * 1e6) as u64
        }
        fn speed(&self) -> f64 {
            self.speed
        }
        fn sleep(&self, d: Duration) {
            std::thread::sleep(d);
        }
    }

    /// Recording or pinging a replay makes no sense; refuse politely.
    fn refuse(c: Control) {
        const NO: &str = "Not available during a replay.";
        match c {
            Control::SetRecording(_) => {}
            Control::ComparePing(reply) => {
                let _ = reply.send(Err(NO.into()));
            }
            Control::CompareGamePings(reply) => {
                let _ = reply.send(Err(NO.into()));
            }
            Control::TestRouteProbe(reply) => {
                let _ = reply.send(Err(NO.into()));
            }
        }
    }

    /// Plays a trace back as if it were live, paced by `clock`.
    pub fn run_replay(
        path: &Path,
        clock: &impl Clock,
        control: &Receiver<Control>,
        emit: &impl Fn(Snapshot),
    ) {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                return emit(Snapshot::unavailable(format!(
                    "can't open replay trace: {e}"
                )));
            }
        };
        let mut source = ReplaySource::new(BufReader::new(file));
        let mut pipe = Pipeline::new(None);
        // Snapshots every 250 ms of wall time = 250 ms × speed of trace time.
        let every_us = (EMIT_EVERY_US as f64 * clock.speed()) as u64;
        let mut error = None;

        loop {
            while let Ok(c) = control.try_recv() {
                refuse(c);
            }

            match source.next() {
                Ok(Next::Packet(h)) => {
                    // Wait until this packet's moment, emitting along the way.
                    while clock.now_us() < h.timestamp_us {
                        if pipe.due(clock.now_us(), every_us) {
                            // Placeholder addresses can't be pinged: passive only.
                            pipe.track_session(clock.now_us(), |_| false);
                            emit(pipe.snapshot(
                                SourceKind::Replay,
                                GameStatus::Running,
                                0,
                                clock.now_us(),
                                None,
                            ));
                        }
                        clock.sleep(Duration::from_millis(10));
                    }
                    if let Err(e) = pipe.observe(&h) {
                        error = Some(e);
                        break;
                    }
                }
                Ok(Next::Idle) => {}
                Ok(Next::End) => break,
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
            if pipe.due(clock.now_us(), every_us) {
                pipe.track_session(clock.now_us(), |_| false);
                emit(pipe.snapshot(
                    SourceKind::Replay,
                    GameStatus::Running,
                    0,
                    clock.now_us(),
                    None,
                ));
            }
        }

        // Keep the final state on screen; flows age out as trace time moves on.
        loop {
            emit(pipe.snapshot(
                SourceKind::Replay,
                GameStatus::ReplayEnded,
                0,
                clock.now_us(),
                error.clone(),
            ));
            clock.sleep(EMIT_EVERY);
            match control.try_recv() {
                Ok(c) => refuse(c),
                Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {}
            }
        }
    }
}

#[cfg(windows)]
mod live {
    use std::net::IpAddr;
    use std::path::Path;
    use std::sync::mpsc::TryRecvError;
    use std::time::{Duration, Instant};

    use efsp_core::capture::{CaptureError, Next, PacketSource};
    use efsp_core::cascade::{PROBE_INTERVAL_US, Probe, ProbeResult};
    use efsp_core::ports::{GameState, POLL_INTERVAL_MS, PortWatcher};

    use super::*;
    use crate::live_source::LiveSource;
    use crate::port_watcher::WindowsPortWatcher;

    /// After this many device errors in a row, reopen the adapter.
    const REOPEN_AFTER_ERRORS: u32 = 5;
    /// How often to re-pick the match connection.
    const TRACK_EVERY: Duration = Duration::from_millis(100);

    pub(super) fn unix_now_us() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
    }

    /// The single probe budget for everything this app sends (≤ 5
    /// probes/s). Every sender draws from it, on a monotonic clock.
    pub(super) struct ProbeBudget {
        last: Option<Instant>,
    }

    impl ProbeBudget {
        fn new() -> Self {
            Self { last: None }
        }

        /// True if `interval` has passed since the last probe sent.
        fn ready(&self, interval: Duration) -> bool {
            self.last.is_none_or(|t| t.elapsed() >= interval)
        }

        /// Records that a probe was just sent.
        fn mark(&mut self) {
            self.last = Some(Instant::now());
        }
    }

    /// Separate slots so fixing one problem doesn't hide another, and each
    /// clears itself once things work again.
    #[derive(Default)]
    pub(super) struct Errors {
        capture: Option<String>,
        ports: Option<String>,
        pub(super) dev: Option<String>,
    }

    impl Errors {
        fn summary(&self) -> Option<String> {
            let all: Vec<&str> = [&self.capture, &self.ports, &self.dev]
                .into_iter()
                .flatten()
                .map(String::as_str)
                .collect();
            (!all.is_empty()).then(|| all.join(" "))
        }
    }

    pub fn run_live(
        control: &Receiver<Control>,
        emit: &impl Fn(Snapshot),
        traces_dir: &Path,
        history: Option<Arc<History>>,
        geo: Geo,
    ) {
        let mut source = match LiveSource::open() {
            Ok(s) => s,
            Err(e) => return emit(Snapshot::unavailable(e.to_string())),
        };
        let mut watcher = WindowsPortWatcher;
        let mut pipe = Pipeline::new(history).with_geo(geo);
        let poll_every = Duration::from_millis(POLL_INTERVAL_MS);
        let mut last_poll: Option<Instant> = None;
        let mut last_track: Option<Instant> = None;
        let mut game = GameState::NotRunning;
        let mut errors = Errors::default();
        let mut device_errors = 0;
        let mut budget = ProbeBudget::new();
        // Probe results come back from short-lived probe threads.
        let (probe_tx, probe_rx) = mpsc::channel::<(u32, Probe, ProbeResult)>();
        #[cfg(feature = "dev-tools")]
        let mut dev = dev::Dev::new(traces_dir);
        #[cfg(not(feature = "dev-tools"))]
        let _ = traces_dir;

        loop {
            while let Ok((generation, probe, result)) = probe_rx.try_recv() {
                if generation == pipe.session_gen
                    && let Some(session) = &mut pipe.session
                {
                    session.on_result(probe, result);
                }
            }

            #[cfg(feature = "dev-tools")]
            loop {
                match control.try_recv() {
                    Ok(c) => dev.handle(c, &mut pipe, &mut errors),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            // Release builds accept no requests; only notice the app closing.
            #[cfg(not(feature = "dev-tools"))]
            match control.try_recv() {
                Ok(never) => match never {},
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }
            #[cfg(feature = "dev-tools")]
            dev.poll(&mut pipe, &mut errors, unix_now_us());

            if last_poll.is_none_or(|t| t.elapsed() >= poll_every) {
                last_poll = Some(Instant::now());
                match watcher.poll() {
                    Ok(state) => {
                        game = state;
                        errors.ports = None;
                    }
                    Err(e) => errors.ports = Some(e),
                }
                let empty = Default::default();
                let ports = match &game {
                    GameState::Running { ports } => ports,
                    GameState::NotRunning => &empty,
                };
                if let Err(e) = source.set_ports(ports) {
                    errors.capture = Some(e.to_string());
                }
            }

            match source.next() {
                Ok(next) => {
                    device_errors = 0;
                    errors.capture = None;
                    if let Next::Packet(h) = next
                        && let Err(e) = pipe.observe(&h)
                    {
                        // Only the dev-build recorder can fail here; it must
                        // not stop monitoring.
                        #[cfg(feature = "dev-tools")]
                        {
                            pipe.recorder = None;
                        }
                        errors.dev = Some(format!("Recording stopped: {e}"));
                    }
                }
                Err(CaptureError::Device(e)) => {
                    errors.capture = Some(e);
                    device_errors += 1;
                    std::thread::sleep(Duration::from_millis(500));
                    if device_errors >= REOPEN_AFTER_ERRORS {
                        // e.g. the adapter was reset or Wi-Fi reconnected.
                        if let Ok(fresh) = LiveSource::open() {
                            source = fresh;
                            last_poll = None; // re-apply the port filter now
                            device_errors = 0;
                        }
                    }
                }
                Err(e) => errors.capture = Some(e.to_string()),
            }

            let now = unix_now_us();
            if last_track.is_none_or(|t| t.elapsed() >= TRACK_EVERY) {
                last_track = Some(Instant::now());
                // IPv6 targets aren't probed yet: passive numbers only.
                pipe.track_session(now, |c| c.remote.get().is_ipv4());
            }

            // One budget for all probes. In dev builds a running ping.exe
            // check (1/s of its own) slows ours to 4/s, and the game-ping
            // experiment pauses match probing entirely.
            #[cfg(feature = "dev-tools")]
            let (interval, paused) = {
                dev.set_match_active(pipe.session.is_some());
                (dev.probe_interval(), dev.experiment_running())
            };
            #[cfg(not(feature = "dev-tools"))]
            let (interval, paused) = (Duration::from_micros(PROBE_INTERVAL_US), false);
            if !paused
                && budget.ready(interval)
                && let Some(session) = &mut pipe.session
                && let Some((probe, IpAddr::V4(target))) = session.next_probe(now)
            {
                budget.mark();
                send_probe(pipe.session_gen, probe, target, &probe_tx);
            }

            if pipe.due(now, EMIT_EVERY_US) {
                let (status, ports) = match &game {
                    GameState::Running { ports } => (GameStatus::Running, ports.len()),
                    GameState::NotRunning => (GameStatus::NotRunning, 0),
                };
                emit(pipe.snapshot(SourceKind::Live, status, ports, now, errors.summary()));
            }
        }
    }

    /// Runs one blocking probe on its own thread (≤ 5/s, each ≤ 1 s, so at
    /// most a handful at once) and reports back on `tx`.
    fn send_probe(
        generation: u32,
        probe: Probe,
        target: std::net::Ipv4Addr,
        tx: &Sender<(u32, Probe, ProbeResult)>,
    ) {
        let thread_tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("efsp-probe".into())
            .spawn(move || {
                let result = crate::icmp::probe(target, probe.ttl);
                let _ = thread_tx.send((generation, probe, result));
            });
        if spawned.is_err() {
            let _ = tx.send((generation, probe, ProbeResult::Lost));
        }
    }

    /// Developer tools for the live loop. Compiled only with `dev-tools`.
    #[cfg(feature = "dev-tools")]
    mod dev {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicBool, Ordering};

        use efsp_core::cascade::Method;
        use efsp_core::pingexe::Comparison;

        use super::*;

        type CompareReply = Sender<Result<Comparison, String>>;

        struct PingExeDone {
            generation: u32,
            reply: CompareReply,
            output: Result<String, String>,
        }

        pub(in super::super) struct Dev {
            traces_dir: PathBuf,
            ping_tx: Sender<PingExeDone>,
            ping_rx: Receiver<PingExeDone>,
            ping_exe_running: bool,
            /// Set while the experiment thread runs; it clears it on exit.
            experiment_running: Arc<AtomicBool>,
            /// Tells the experiment to stop if a match starts.
            match_active: Arc<AtomicBool>,
        }

        impl Dev {
            pub fn new(traces_dir: &Path) -> Self {
                let (ping_tx, ping_rx) = mpsc::channel();
                Self {
                    traces_dir: traces_dir.to_path_buf(),
                    ping_tx,
                    ping_rx,
                    ping_exe_running: false,
                    experiment_running: Arc::new(AtomicBool::new(false)),
                    match_active: Arc::new(AtomicBool::new(false)),
                }
            }

            pub fn set_match_active(&self, active: bool) {
                self.match_active.store(active, Ordering::Relaxed);
            }

            pub fn experiment_running(&self) -> bool {
                self.experiment_running.load(Ordering::Relaxed)
            }

            /// ping.exe sends 1/s of its own, so ours drop to 4/s meanwhile.
            pub fn probe_interval(&self) -> Duration {
                if self.ping_exe_running {
                    Duration::from_millis(250)
                } else {
                    Duration::from_micros(PROBE_INTERVAL_US)
                }
            }

            pub fn handle(&mut self, c: Control, pipe: &mut Pipeline, errors: &mut Errors) {
                match c {
                    Control::SetRecording(on) => {
                        errors.dev = set_recording(pipe, on, &self.traces_dir).err();
                    }
                    Control::ComparePing(reply) => self.start_ping_exe(pipe, reply),
                    Control::CompareGamePings(reply) => self.start_experiment(pipe, reply),
                    Control::TestRouteProbe(reply) => self.test_route_probe(pipe, reply),
                }
            }

            /// Finishes a ping.exe check whose output has arrived.
            pub fn poll(&mut self, pipe: &mut Pipeline, _errors: &mut Errors, now_us: u64) {
                while let Ok(done) = self.ping_rx.try_recv() {
                    self.ping_exe_running = false;
                    let result = match done.output {
                        Err(e) => Err(e),
                        Ok(_) if done.generation != pipe.session_gen => {
                            Err("The match connection changed during the check.".into())
                        }
                        Ok(_)
                            if pipe
                                .session
                                .as_ref()
                                .is_none_or(|s| s.method() != Method::Icmp) =>
                        {
                            Err("The method changed during the check; try again.".into())
                        }
                        Ok(output) => {
                            let ours = pipe
                                .session
                                .as_mut()
                                .and_then(|s| s.view(now_us).ping)
                                .and_then(|p| p.median_ms);
                            Ok(efsp_core::pingexe::compare(ours, &output))
                        }
                    };
                    let _ = done.reply.send(result);
                }
            }

            /// Runs `ping -n 10` against the current ICMP target on a thread.
            /// The address goes to ping.exe as an argument and nowhere else.
            fn start_ping_exe(&mut self, pipe: &Pipeline, reply: CompareReply) {
                use std::os::windows::process::CommandExt;

                if self.ping_exe_running {
                    let _ = reply.send(Err("A ping check is already running.".into()));
                    return;
                }
                let Some(session) = &pipe.session else {
                    let _ = reply.send(Err("No match connection is being measured.".into()));
                    return;
                };
                if session.method() != Method::Icmp {
                    let _ =
                        reply.send(Err("Only possible while measuring with ping (ICMP).".into()));
                    return;
                }
                self.ping_exe_running = true;
                let target = session.target_ip().to_string();
                let generation = pipe.session_gen;
                let tx = self.ping_tx.clone();
                std::thread::spawn(move || {
                    // Full System32 path, so a stray ping.exe on PATH is never
                    // used. %SystemRoot% can be changed by the environment;
                    // acceptable for a dev-only tool.
                    let root =
                        std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
                    let exe = PathBuf::from(root).join(r"System32\PING.EXE");
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    let output = std::process::Command::new(exe)
                        .args(["-n", "10", "-w", "1000", &target])
                        .creation_flags(CREATE_NO_WINDOW)
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                        .map_err(|e| format!("couldn't run ping.exe: {e}"));
                    let _ = tx.send(PingExeDone {
                        generation,
                        reply,
                        output,
                    });
                });
            }

            /// Sends one probe with TTL 1. It dies at the first router (your
            /// home router), which should answer "TTL expired": that is the
            /// reply route estimates depend on. Between matches only, so it
            /// can't add to match probing. The destination is a reserved
            /// documentation address; the probe never gets that far.
            fn test_route_probe(&self, pipe: &Pipeline, reply: Sender<Result<String, String>>) {
                if pipe.session.is_some() || self.experiment_running() {
                    let _ = reply.send(Err("Run this between matches.".into()));
                    return;
                }
                std::thread::spawn(move || {
                    let target = std::net::Ipv4Addr::new(192, 0, 2, 1);
                    let text = match crate::icmp::probe(target, Some(1)) {
                        ProbeResult::TtlExpired { rtt_us } => format!(
                            "Works: the first router answered \"TTL expired\" in {} ms, so route estimates can work.",
                            rtt_us / 1000
                        ),
                        ProbeResult::Lost => "No answer from the first router. Route estimates may not work on this network (or the reply isn't being read).".into(),
                        ProbeResult::EchoReply { .. } => "Unexpected echo reply; please report this.".into(),
                    };
                    let _ = reply.send(Ok(text));
                });
            }

            /// Experiment: ICMP-probes the 5 nearest game-pinged servers 5
            /// times each (one per 200 ms) and compares medians. Only between
            /// matches; match probing waits while it runs, and it stops as
            /// soon as a match starts, so all probing stays ≤ 5/s.
            fn start_experiment(
                &mut self,
                pipe: &Pipeline,
                reply: Sender<Result<Vec<ServerComparison>, String>>,
            ) {
                if self.experiment_running() {
                    let _ = reply.send(Err("The comparison is already running.".into()));
                    return;
                }
                if pipe.session.is_some() {
                    let _ = reply.send(Err("Run this between matches.".into()));
                    return;
                }
                let nearest = pipe.game_pings.nearest(5);
                if nearest.is_empty() {
                    let _ = reply.send(Err(
                        "No game pings seen yet. Leave eFootball on the menu for ~10 s.".into(),
                    ));
                    return;
                }
                self.experiment_running.store(true, Ordering::Relaxed);
                let running = Arc::clone(&self.experiment_running);
                let match_active = Arc::clone(&self.match_active);
                std::thread::spawn(move || {
                    /// Clears the flag however the thread ends.
                    struct Done(Arc<AtomicBool>);
                    impl Drop for Done {
                        fn drop(&mut self) {
                            self.0.store(false, Ordering::Relaxed);
                        }
                    }
                    let _done = Done(running);

                    let mut results = Vec::new();
                    for (ip, _) in &nearest {
                        let mut probes = Vec::new();
                        for _ in 0..5 {
                            if match_active.load(Ordering::Relaxed) {
                                let _ = reply.send(Err(
                                    "A match started, so the comparison stopped (5 probes/s cap)."
                                        .into(),
                                ));
                                return;
                            }
                            std::thread::sleep(Duration::from_micros(PROBE_INTERVAL_US));
                            probes.push(match ip {
                                IpAddr::V4(v4) => match crate::icmp::probe(*v4, None) {
                                    ProbeResult::EchoReply { rtt_us } => Some(rtt_us),
                                    _ => None,
                                },
                                IpAddr::V6(_) => None,
                            });
                        }
                        results.push(probes);
                    }
                    let game: Vec<f64> = nearest.iter().map(|(_, ms)| *ms).collect();
                    let _ = reply.send(Ok(efsp_core::gameping::compare_rows(&game, &results)));
                });
            }
        }

        fn set_recording(pipe: &mut Pipeline, on: bool, dir: &Path) -> Result<(), String> {
            use efsp_core::trace::TraceWriter;
            use std::fs::File;
            use std::io::BufWriter;

            if !on {
                return match pipe.recorder.take() {
                    Some((writer, _)) => writer.finish().map(drop).map_err(|e| e.to_string()),
                    None => Ok(()),
                };
            }
            if pipe.recorder.is_some() {
                return Ok(());
            }
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let file_name = format!("efsp-trace-{}.jsonl", unix_now_us() / 1_000_000);
            let file = File::create(dir.join(&file_name)).map_err(|e| e.to_string())?;
            let writer = TraceWriter::new(BufWriter::new(file)).map_err(|e| e.to_string())?;
            pipe.recorder = Some((
                writer,
                Recording {
                    file_name,
                    packets: 0,
                },
            ));
            Ok(())
        }
    }
}

#[cfg(all(test, feature = "dev-tools"))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::Path;
    use std::time::Duration;

    use super::replay::{Clock, run_replay};
    use super::*;

    /// Advances instantly when asked to sleep: deterministic on every OS.
    struct FakeClock {
        wall_us: Cell<u64>,
        speed: f64,
    }

    impl Clock for FakeClock {
        fn now_us(&self) -> u64 {
            (self.wall_us.get() as f64 * self.speed) as u64
        }
        fn speed(&self) -> f64 {
            self.speed
        }
        fn sleep(&self, d: Duration) {
            self.wall_us.set(self.wall_us.get() + d.as_micros() as u64);
        }
    }

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/core/tests/fixtures/traces/synthetic-lobby-then-match.jsonl")
    }

    #[test]
    fn replay_streams_flows_and_then_reports_the_end() {
        let clock = FakeClock {
            wall_us: Cell::new(0),
            speed: 1.0,
        };
        let snapshots = RefCell::new(Vec::new());
        let (tx, rx) = mpsc::channel::<Control>();
        // Keep the replay running after the trace ends until the match has
        // gone idle and become the "last match"; then drop the sender, which
        // lets the "replay ended" loop return.
        let tx = RefCell::new(Some(tx));
        run_replay(&fixture(), &clock, &rx, &|s: Snapshot| {
            if s.game == GameStatus::ReplayEnded && s.last_match.is_some() {
                tx.borrow_mut().take();
            }
            snapshots.borrow_mut().push(s);
        });

        let snapshots = snapshots.into_inner();
        // 30 s trace plus ~3 s until the match goes idle, at 4 snapshots/s.
        assert!(
            (125..=140).contains(&snapshots.len()),
            "{}",
            snapshots.len()
        );
        assert!(snapshots.iter().all(|s| s.source == SourceKind::Replay));
        let first_steady = snapshots
            .iter()
            .position(|s| s.flows.iter().any(|f| f.sustained))
            .expect("the match flow should become sustained");
        // The match starts at 10 s and is steady after 1 s: snapshot ~45.
        assert!((43..=46).contains(&first_steady), "{first_steady}");
        let last = snapshots.last().unwrap();
        assert_eq!(last.game, GameStatus::ReplayEnded);
        assert_eq!(last.error, None);

        // During the match the snapshot measures it, passively (replays can't
        // be pinged), and the numbers point at a flow in the list.
        let during = &snapshots[60]; // ~15 s: mid-match
        let m = during
            .measurement
            .as_ref()
            .expect("match should be measured");
        assert_eq!(m.method, efsp_core::cascade::Method::Passive);
        assert!(m.ping.is_none());
        assert!(
            (25.0..=35.0).contains(&m.passive.rate_pps),
            "{:?}",
            m.passive
        );
        assert!(during.flows.iter().any(|f| f.id == m.flow_id));
        // Before the match there's nothing to measure.
        assert!(snapshots[20].measurement.is_none());
        // The live graph gets up to the last minute of points.
        assert!(!m.recent.is_empty());

        // After the match (ends at 30 s, idle 3 s later) it is the last match.
        let last = snapshots.last().unwrap();
        assert!(last.measurement.is_none());
        let summary = last.last_match.as_ref().expect("last match kept");
        assert_eq!(summary.flow_id, m.flow_id);
    }

    #[test]
    fn missing_trace_is_reported_not_fatal() {
        let clock = FakeClock {
            wall_us: Cell::new(0),
            speed: 1.0,
        };
        let (_tx, rx) = mpsc::channel();
        let got = RefCell::new(None);
        run_replay(Path::new("/nope.jsonl"), &clock, &rx, &|s| {
            *got.borrow_mut() = Some(s);
        });
        let s = got.into_inner().unwrap();
        assert_eq!(s.source, SourceKind::Unavailable);
        assert!(s.error.unwrap().contains("can't open replay trace"));
    }
}

/// Pipeline lifecycle tests: no Tauri, no OS, any build.
#[cfg(test)]
mod pipeline_tests {
    use efsp_core::capture::{Direction, Protocol, RemoteAddr};

    use super::*;

    const SEC: u64 = 1_000_000;

    /// Two-way UDP at `pps` between `from` and `to` (µs).
    fn traffic(remote: &str, port: u16, from: u64, to: u64, pps: u64) -> Vec<PacketHeader> {
        let mut out = Vec::new();
        let mut t = from;
        let mut i = 0;
        while t < to {
            out.push(PacketHeader {
                timestamp_us: t,
                direction: if i % 2 == 0 {
                    Direction::Outbound
                } else {
                    Direction::Inbound
                },
                protocol: Protocol::Udp,
                local_port: port,
                remote: RemoteAddr::new(remote.parse().unwrap()),
                wire_len: 120,
                ttl: Some(56),
            });
            t += SEC / pps;
            i += 1;
        }
        out
    }

    /// Drives the pipeline like the live loop: packets in time order, and a
    /// track + snapshot every 250 ms. Returns every snapshot.
    fn run(pipe: &mut Pipeline, packets: Vec<PacketHeader>, until: u64) -> Vec<Snapshot> {
        run_with_ping(pipe, packets, until, None)
    }

    /// Like `run`, but when `ping_us` is set, probes are sent and answered
    /// at once with that RTT (standing in for the Windows ICMP prober).
    fn run_with_ping(
        pipe: &mut Pipeline,
        mut packets: Vec<PacketHeader>,
        until: u64,
        ping_us: Option<u32>,
    ) -> Vec<Snapshot> {
        packets.sort_by_key(|p| p.timestamp_us);
        let mut next = packets.into_iter().peekable();
        let mut snapshots = Vec::new();
        let mut now = 0;
        while now <= until {
            while let Some(p) = next.next_if(|p| p.timestamp_us <= now) {
                pipe.observe(&p).unwrap();
            }
            pipe.track_session(now, |_| ping_us.is_some());
            if let (Some(rtt_us), Some(session)) = (ping_us, &mut pipe.session)
                && let Some((probe, _)) = session.next_probe(now)
            {
                session.on_result(probe, efsp_core::cascade::ProbeResult::EchoReply { rtt_us });
            }
            snapshots.push(pipe.snapshot(SourceKind::Replay, GameStatus::Running, 0, now, None));
            now += 250_000;
        }
        snapshots
    }

    #[test]
    fn a_new_match_replaces_the_session_and_the_old_one_becomes_last_match() {
        let mut pipe = Pipeline::new(None);
        // Match A 0–10 s, silence, match B from 20 s.
        let mut packets = traffic("203.0.113.1:5735", 50_000, 0, 10 * SEC, 100);
        packets.extend(traffic("203.0.113.2:5735", 50_001, 20 * SEC, 30 * SEC, 100));
        let snaps = run(&mut pipe, packets, 30 * SEC);

        let a = snaps[20]
            .measurement
            .as_ref()
            .expect("match A measured")
            .flow_id; // 5 s
        let end = snaps.last().unwrap();
        let b = end.measurement.as_ref().expect("match B measured").flow_id;
        assert_ne!(a, b);
        let last = end.last_match.as_ref().expect("match A kept as last match");
        assert_eq!(last.flow_id, a);
        // Its numbers stop where its traffic stopped, not 3 s later.
        assert!(
            (9_000..=10_000).contains(&last.duration_ms),
            "{}",
            last.duration_ms
        );
        // Between the matches, A is shown as the last match.
        assert!(snaps[64].measurement.is_none()); // 16 s
        assert_eq!(snaps[64].last_match.as_ref().unwrap().flow_id, a);
    }

    #[test]
    fn a_short_burst_does_not_replace_the_last_match() {
        let mut pipe = Pipeline::new(None);
        // Match A 0–10 s, then a half-second burst at 20 s: busy enough to be
        // measured, never sustained.
        let mut packets = traffic("203.0.113.1:5735", 50_000, 0, 10 * SEC, 100);
        packets.extend(traffic(
            "203.0.113.9:3478",
            50_002,
            20 * SEC,
            20 * SEC + 500_000,
            60,
        ));
        let snaps = run(&mut pipe, packets, 30 * SEC);

        let a = snaps[20]
            .measurement
            .as_ref()
            .expect("match A measured")
            .flow_id;
        let end = snaps.last().unwrap();
        assert!(end.measurement.is_none());
        assert_eq!(end.last_match.as_ref().expect("still match A").flow_id, a);
    }

    #[test]
    fn a_udp_match_takes_over_from_a_tcp_session() {
        let mut pipe = Pipeline::new(None);
        // A steady TCP flow on a game port from 0 s (a possible TCP match),
        // still running when a UDP match starts at 10 s.
        let mut packets: Vec<PacketHeader> = traffic("203.0.113.7:6000", 50_010, 0, 30 * SEC, 60)
            .into_iter()
            .map(|p| PacketHeader {
                protocol: Protocol::Tcp,
                ..p
            })
            .collect();
        packets.extend(traffic("203.0.113.1:5735", 50_000, 10 * SEC, 30 * SEC, 100));
        let snaps = run(&mut pipe, packets, 20 * SEC);

        let tcp = snaps[30].measurement.as_ref().expect("TCP measured first");
        assert_eq!(tcp.protocol, Protocol::Tcp);
        let udp = snaps
            .last()
            .unwrap()
            .measurement
            .as_ref()
            .expect("measuring");
        assert_eq!(
            udp.protocol,
            Protocol::Udp,
            "UDP took over while TCP still ran"
        );
    }

    /// Home ISPs are peers; nothing else is looked up.
    struct HomeIsp;
    impl efsp_core::classify::GeoLookup for HomeIsp {
        fn country(&self, _: std::net::IpAddr) -> Option<String> {
            Some("CA".into())
        }
        fn network(&self, _: std::net::IpAddr) -> Option<String> {
            Some("Example Home Broadband".into())
        }
    }

    /// An address in a published cloud range (a game server), as "ip:port".
    fn cloud_server() -> String {
        let (addr, _) = efsp_core::classify::ranges_in_area("server:Frankfurt|DE")
            .into_iter()
            .find(|(a, len)| a.is_ipv4() && *len <= 30)
            .unwrap();
        let std::net::IpAddr::V4(v4) = addr else {
            unreachable!()
        };
        format!("{}:5735", std::net::Ipv4Addr::from(u32::from(v4) + 1))
    }

    fn saved(history: &History) -> Vec<efsp_core::history::MatchRecord> {
        history
            .with(|s| {
                s.list(&crate::store::ListQuery {
                    since: None,
                    include_offers: true,
                    limit: 50,
                    offset: 0,
                    ..crate::store::ListQuery::default()
                })
            })
            .unwrap()
    }

    #[test]
    fn a_short_server_session_before_a_peer_match_is_setup_not_an_offer() {
        let history = Arc::new(History::in_memory());
        let mut pipe = Pipeline::new(Some(Arc::clone(&history))).with_geo(Arc::new(HomeIsp));
        // 6 s with a game server, then the peer match from 10 s to 80 s.
        let mut packets = traffic(&cloud_server(), 50_000, 0, 6 * SEC, 100);
        packets.extend(traffic(
            "198.51.100.7:6000",
            50_001,
            10 * SEC,
            80 * SEC,
            100,
        ));
        run(&mut pipe, packets, 120 * SEC);
        let records = saved(&history);
        assert_eq!(records.len(), 1, "{records:?}");
        assert!(matches!(
            records[0].endpoint,
            Some(efsp_core::classify::Endpoint::Peer { .. })
        ));
    }

    #[test]
    fn a_short_server_session_with_no_peer_match_after_is_an_offer() {
        let history = Arc::new(History::in_memory());
        let mut pipe = Pipeline::new(Some(Arc::clone(&history))).with_geo(Arc::new(HomeIsp));
        let snaps = run(
            &mut pipe,
            traffic(&cloud_server(), 50_000, 0, 8 * SEC, 100),
            60 * SEC,
        );
        let records = saved(&history);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, efsp_core::history::Kind::Offer);
        // The snapshot says something was saved, so History reloads.
        assert_eq!(snaps.first().unwrap().saved, 0);
        assert_eq!(snaps.last().unwrap().saved, 1);
    }

    #[test]
    fn played_matches_are_saved() {
        let history = Arc::new(History::in_memory());
        let mut pipe = Pipeline::new(Some(Arc::clone(&history)));
        // Three played matches (> 60 s): two with the same server, then another.
        let mut packets = traffic("203.0.113.1:5735", 50_000, 0, 62 * SEC, 100);
        packets.extend(traffic(
            "203.0.113.1:5735",
            50_001,
            70 * SEC,
            132 * SEC,
            100,
        ));
        packets.extend(traffic(
            "203.0.113.2:5735",
            50_002,
            140 * SEC,
            202 * SEC,
            100,
        ));
        let snaps = run_with_ping(&mut pipe, packets, 210 * SEC, Some(30_000));

        let saved = history
            .with(|s| {
                s.list(&crate::store::ListQuery {
                    since: None,
                    include_offers: true,
                    limit: 10,
                    offset: 0,
                    ..crate::store::ListQuery::default()
                })
            })
            .unwrap();
        assert_eq!(saved.len(), 3);
        assert!(
            saved
                .iter()
                .all(|r| r.kind == efsp_core::history::Kind::Match)
        );
        assert!(saved.iter().all(|r| r.median_ms == Some(30.0)));
        // The card measured each one while it ran.
        assert!(snaps[400].measurement.is_some()); // 100 s, second match
    }
}
