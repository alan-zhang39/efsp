import { invoke } from "@tauri-apps/api/core";
import { type Endpoint, parseEndpointOrNull } from "./endpoint";
import { parsePoints, type Method, type Point } from "./monitor";

/** Mirrors `efsp_core::history::MatchRecord`. No addresses, no fingerprints. */
export interface MatchRecord {
  id: number;
  /** Unix seconds (UTC). */
  startedAt: number;
  durationMs: number;
  kind: "offer" | "match";
  method: Method;
  medianMs: number | null;
  p95Ms: number | null;
  jitterMs: number | null;
  lossPct: number | null;
  stalls: number;
  bars: number | null;
  ratePps: number;
  spark: (number | null)[];
  /** Where the match was; null for matches saved before locations were recorded. */
  endpoint: Endpoint | null;
  protocol: "udp" | "tcp";
}

export interface HistoryPage {
  records: MatchRecord[];
  total: number;
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null;
function fail(what: string, v: unknown): never {
  throw new Error(`Unexpected ${what} from the app: ${JSON.stringify(v)}`);
}
function num(o: Obj, k: string): number {
  const v = o[k];
  return typeof v === "number" && Number.isFinite(v) ? v : fail(k, v);
}
function numOrNull(o: Obj, k: string): number | null {
  return o[k] === null || o[k] === undefined ? null : num(o, k);
}

const METHODS = ["icmp", "routeSearch", "routeEstimate", "passive"] as const;

export function parseRecord(v: unknown): MatchRecord {
  if (!isObj(v)) fail("record", v);
  const method = METHODS.find((m) => m === v.method) ?? fail("method", v.method);
  const kind = v.kind === "offer" || v.kind === "match" ? v.kind : fail("kind", v.kind);
  const spark = Array.isArray(v.spark) ? v.spark : [];
  return {
    id: num(v, "id"),
    startedAt: num(v, "startedAt"),
    durationMs: num(v, "durationMs"),
    kind,
    method,
    medianMs: numOrNull(v, "medianMs"),
    p95Ms: numOrNull(v, "p95Ms"),
    jitterMs: numOrNull(v, "jitterMs"),
    lossPct: numOrNull(v, "lossPct"),
    stalls: num(v, "stalls"),
    bars: numOrNull(v, "bars"),
    ratePps: num(v, "ratePps"),
    spark: spark.map((x) => (typeof x === "number" ? x : null)),
    endpoint: parseEndpointOrNull(v.endpoint),
    protocol:
      v.protocol === "tcp" ? "tcp" : v.protocol === "udp" ? "udp" : fail("protocol", v.protocol),
  };
}

export type HistorySort = "when" | "ping" | "length";

export interface ListArgs {
  sinceDays: number | null;
  includeOffers: boolean;
  limit: number;
  offset: number;
  /** Only server or only peer-to-peer matches. */
  kind: "server" | "peer" | null;
  /** Only servers in cities whose name contains this. */
  city: string | null;
  sort: HistorySort;
  ascending: boolean;
}

export async function listHistory(args: ListArgs): Promise<HistoryPage> {
  const v = await invoke<unknown>("history_list", { query: args });
  if (!isObj(v) || !Array.isArray(v.records)) fail("history page", v);
  return {
    records: v.records.map(parseRecord),
    total: num(v, "total"),
  };
}

export async function historyDetail(
  id: number,
): Promise<{ record: MatchRecord; timeline: Point[] } | null> {
  const v = await invoke<unknown>("history_detail", { id });
  if (v === null) return null;
  if (!isObj(v)) fail("history detail", v);
  return { record: parseRecord(v.record), timeline: parsePoints(v.timeline) };
}

/** Opens Windows' Save dialog; false if cancelled. */
export async function exportCsv(): Promise<boolean> {
  return (await invoke<unknown>("history_export_csv")) === true;
}

/** Asks for confirmation natively; null if cancelled, else how many deleted. */
export async function deleteAllHistory(): Promise<number | null> {
  const v = await invoke<unknown>("history_delete_all");
  return typeof v === "number" ? v : null;
}
