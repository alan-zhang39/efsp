import { Channel, invoke } from "@tauri-apps/api/core";
import { type Endpoint, parseEndpoint, parseEndpointOrNull } from "./endpoint";

/** Mirrors `efsp_core::flow::FlowView`. No IP addresses, by design. */
export interface FlowView {
  id: number;
  remoteLabel: number;
  protocol: "udp" | "tcp";
  localPort: number;
  remotePort: number;
  ageMs: number;
  idleMs: number;
  packetsIn: number;
  packetsOut: number;
  bytesIn: number;
  bytesOut: number;
  ratePps: number;
  sustained: boolean;
  idle: boolean;
  /** Server region, peer country, local or unclassified. */
  endpoint: Endpoint;
}

/** Mirrors `efsp_core::measure::ProbeView`. */
export interface PingView {
  lastMs: number | null;
  medianMs: number | null;
  p95Ms: number | null;
  jitterMs: number;
  lossPct: number;
  probes: number;
}

/** Mirrors `efsp_core::measure::PassiveView`. */
export interface PassiveView {
  ratePps: number;
  jitterMs: number;
  stalls: number;
  longestGapMs: number;
}

export type Method = "icmp" | "routeSearch" | "routeEstimate" | "passive";

/** Mirrors `efsp_core::session::Point`: one graph point per ~250 ms. */
export interface Point {
  tMs: number;
  rttMs: number | null;
  /** Probe jitter; null without probe numbers (passive). */
  jitterMs: number | null;
  lost: number;
  stalls: number;
  ratePps: number;
}

/** Mirrors `efsp_core::session::MeasurementView`. No addresses. */
export interface Measurement {
  flowId: number;
  method: Method;
  ping: PingView | null;
  passive: PassiveView;
  /** efsp's bars, 1–5; null without a ping number. */
  bars: number | null;
  durationMs: number;
  /** The last 60 s of points, for the live graph. */
  recent: Point[];
  /** False when the target was never probed at all (replay, IPv6). */
  probing: boolean;
  /** Server region or peer country; null until classified. */
  endpoint: Endpoint | null;
  /** "tcp" for a possible TCP match. */
  protocol: "udp" | "tcp";
}

/** Mirrors `efsp_core::gameping::GamePingsView` (dev builds). */
export interface GamePings {
  servers: number;
  samples: number;
  nearestMs: number | null;
  typicalMs: number | null;
  /** The same pings grouped by region, fastest first. */
  regions: RegionPing[];
}

/** Mirrors `efsp_core::gameping::RegionPing`. */
export interface RegionPing {
  endpoint: Endpoint;
  servers: number;
  bestMs: number;
  bars: number;
}

/** Mirrors `efsp_core::gameping::ServerComparison`. Numbered, no addresses. */
export interface ServerComparison {
  server: number;
  gameMs: number;
  icmpMs: number | null;
  icmpReplies: number;
  differenceMs: number | null;
}

/** Mirrors `monitor::Snapshot` in src-tauri/src/monitor.rs. */
export interface Snapshot {
  source: "live" | "replay" | "unavailable";
  game: "notRunning" | "running" | "replayEnded";
  gamePorts: number;
  flows: FlowView[];
  recording: { fileName: string; packets: number } | null;
  measurement: Measurement | null;
  /** The previous match, kept until the next one. */
  lastMatch: Measurement | null;
  /** The game's own background pings, by region (Servers page). */
  gamePings: GamePings | null;
  /** Matches and offers saved to history this run; it changing means reload. */
  saved: number;
  /** Possible TCP matches seen this run; dev builds only (else null). */
  tcpMatches: number | null;
  /** True only in dev-tools builds (trace recorder available). */
  devTools: boolean;
  error: string | null;
}

type Obj = Record<string, unknown>;

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null;
}

function fail(what: string, v: unknown): never {
  throw new Error(`Unexpected ${what} from the app: ${JSON.stringify(v)}`);
}

function num(o: Obj, key: string): number {
  const v = o[key];
  return typeof v === "number" && Number.isFinite(v) ? v : fail(key, v);
}

function bool(o: Obj, key: string): boolean {
  const v = o[key];
  return typeof v === "boolean" ? v : fail(key, v);
}

function oneOf<T extends string>(o: Obj, key: string, options: readonly T[]): T {
  const v = o[key];
  return options.find((opt) => opt === v) ?? fail(key, v);
}

function parseFlow(v: unknown): FlowView {
  if (!isObj(v)) fail("flow", v);
  return {
    id: num(v, "id"),
    remoteLabel: num(v, "remoteLabel"),
    protocol: oneOf(v, "protocol", ["udp", "tcp"] as const),
    localPort: num(v, "localPort"),
    remotePort: num(v, "remotePort"),
    ageMs: num(v, "ageMs"),
    idleMs: num(v, "idleMs"),
    packetsIn: num(v, "packetsIn"),
    packetsOut: num(v, "packetsOut"),
    bytesIn: num(v, "bytesIn"),
    bytesOut: num(v, "bytesOut"),
    ratePps: num(v, "ratePps"),
    sustained: bool(v, "sustained"),
    idle: bool(v, "idle"),
    endpoint: parseEndpoint(v.endpoint),
  };
}

function numOrNull(o: Obj, key: string): number | null {
  return o[key] === null ? null : num(o, key);
}

function parsePoint(v: unknown): Point {
  if (!isObj(v)) fail("point", v);
  return {
    tMs: num(v, "tMs"),
    rttMs: numOrNull(v, "rttMs"),
    jitterMs: numOrNull(v, "jitterMs"),
    lost: num(v, "lost"),
    stalls: num(v, "stalls"),
    ratePps: num(v, "ratePps"),
  };
}

export function parsePoints(v: unknown): Point[] {
  if (!Array.isArray(v)) fail("points", v);
  return v.map(parsePoint);
}

function parseMeasurement(v: unknown): Measurement | null {
  if (v === null) return null;
  if (!isObj(v)) fail("measurement", v);
  const { ping, passive } = v;
  if (!isObj(passive)) fail("passive", passive);
  let pingView: PingView | null = null;
  if (ping !== null) {
    if (!isObj(ping)) fail("ping", ping);
    pingView = {
      lastMs: numOrNull(ping, "lastMs"),
      medianMs: numOrNull(ping, "medianMs"),
      p95Ms: numOrNull(ping, "p95Ms"),
      jitterMs: num(ping, "jitterMs"),
      lossPct: num(ping, "lossPct"),
      probes: num(ping, "probes"),
    };
  }
  return {
    flowId: num(v, "flowId"),
    bars: numOrNull(v, "bars"),
    durationMs: num(v, "durationMs"),
    recent: parsePoints(v.recent),
    probing: bool(v, "probing"),
    endpoint: parseEndpointOrNull(v.endpoint),
    protocol: oneOf(v, "protocol", ["udp", "tcp"] as const),
    method: oneOf(v, "method", ["icmp", "routeSearch", "routeEstimate", "passive"] as const),
    ping: pingView,
    passive: {
      ratePps: num(passive, "ratePps"),
      jitterMs: num(passive, "jitterMs"),
      stalls: num(passive, "stalls"),
      longestGapMs: num(passive, "longestGapMs"),
    },
  };
}

function parseGamePings(v: unknown): GamePings | null {
  if (v === null || v === undefined) return null;
  if (!isObj(v)) fail("gamePings", v);
  return {
    servers: num(v, "servers"),
    samples: num(v, "samples"),
    nearestMs: numOrNull(v, "nearestMs"),
    typicalMs: numOrNull(v, "typicalMs"),
    regions: Array.isArray(v.regions) ? v.regions.map(parseRegionPing) : fail("regions", v.regions),
  };
}

function parseRegionPing(v: unknown): RegionPing {
  if (!isObj(v)) fail("region", v);
  return {
    endpoint: parseEndpoint(v.endpoint),
    servers: num(v, "servers"),
    bestMs: num(v, "bestMs"),
    bars: num(v, "bars"),
  };
}

/** Validates a snapshot from the Rust side instead of trusting a cast. */
export function parseSnapshot(v: unknown): Snapshot {
  if (!isObj(v)) fail("snapshot", v);
  const { flows, recording, error } = v;
  if (!Array.isArray(flows)) fail("flows", flows);
  let rec: Snapshot["recording"] = null;
  if (recording !== null) {
    if (!isObj(recording) || typeof recording.fileName !== "string") fail("recording", recording);
    rec = { fileName: recording.fileName, packets: num(recording, "packets") };
  }
  if (error !== null && typeof error !== "string") fail("error", error);
  return {
    source: oneOf(v, "source", ["live", "replay", "unavailable"] as const),
    game: oneOf(v, "game", ["notRunning", "running", "replayEnded"] as const),
    gamePorts: num(v, "gamePorts"),
    flows: flows.map(parseFlow),
    recording: rec,
    measurement: parseMeasurement(v.measurement),
    lastMatch: parseMeasurement(v.lastMatch),
    gamePings: parseGamePings(v.gamePings),
    saved: num(v, "saved"),
    // Only dev builds send it.
    tcpMatches: v.tcpMatches === undefined ? null : num(v, "tcpMatches"),
    devTools: bool(v, "devTools"),
    error,
  };
}

/**
 * Starts (or re-attaches to) the monitor. Returns a function that stops this
 * page from receiving updates; the Rust side keeps running.
 */
export async function startMonitor(
  onSnapshot: (s: Snapshot) => void,
  onError: (message: string) => void,
): Promise<() => void> {
  let active = true;
  const channel = new Channel<unknown>();
  channel.onmessage = (message) => {
    if (!active) return;
    try {
      onSnapshot(parseSnapshot(message));
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  };
  await invoke("start_monitor", { onUpdate: channel });
  return () => {
    active = false;
  };
}

/** Mirrors `efsp_core::pingexe::Comparison`. No addresses. */
export interface PingComparison {
  efspMedianMs: number | null;
  pingExeMedianMs: number | null;
  pingExeReplies: number;
  differenceMs: number | null;
  withinTolerance: boolean;
}

export function parseComparison(v: unknown): PingComparison {
  if (!isObj(v)) fail("comparison", v);
  return {
    efspMedianMs: numOrNull(v, "efspMedianMs"),
    pingExeMedianMs: numOrNull(v, "pingExeMedianMs"),
    pingExeReplies: num(v, "pingExeReplies"),
    differenceMs: numOrNull(v, "differenceMs"),
    withinTolerance: bool(v, "withinTolerance"),
  };
}

/** Dev builds: ~10 s cross-check against Windows' own ping.exe. */
export async function compareWithPing(): Promise<PingComparison> {
  return parseComparison(await invoke<unknown>("compare_with_ping"));
}

/** Dev builds, experiment: ICMP vs the game's own pings, nearest servers. */
export async function compareGamePings(): Promise<ServerComparison[]> {
  const v = await invoke<unknown>("compare_game_pings");
  if (!Array.isArray(v)) fail("comparison rows", v);
  return v.map((row: unknown) => {
    if (!isObj(row)) fail("comparison row", row);
    return {
      server: num(row, "server"),
      gameMs: num(row, "gameMs"),
      icmpMs: numOrNull(row, "icmpMs"),
      icmpReplies: num(row, "icmpReplies"),
      differenceMs: numOrNull(row, "differenceMs"),
    };
  });
}

/** Dev builds: one TTL-1 probe; returns a plain-language result. */
export async function testRouteProbe(): Promise<string> {
  const v = await invoke<unknown>("test_route_probe");
  if (typeof v !== "string") fail("route test result", v);
  return v;
}

export async function setRecording(on: boolean): Promise<void> {
  await invoke("set_recording", { on });
}

export async function openTracesFolder(): Promise<void> {
  await invoke("open_traces_folder");
}

/** Dev builds: adds 150 sample matches to history, for trying the UI. */
export async function devSeedHistory(): Promise<number> {
  const v = await invoke<unknown>("dev_seed_history");
  if (typeof v !== "number") fail("sample count", v);
  return v;
}
