import { describe, expect, it } from "vitest";
import { parseRecord } from "./history";
import { DEFAULT_SETTINGS, parseSettings } from "./settings";

describe("parseSettings", () => {
  it("falls back to defaults for anything missing or odd", () => {
    expect(parseSettings(undefined)).toEqual(DEFAULT_SETTINGS);
    const s = parseSettings({ theme: "neon", keepHistoryDays: 7, show: { packets: true } });
    expect(s.theme).toBe("dark");
    expect(s.keepHistoryDays).toBeNull();
    expect(s.show.packets).toBe(true);
    expect(s.show.jitter).toBe(true);
  });

  it("keeps valid values", () => {
    const s = parseSettings({ theme: "system", keepHistoryDays: 90, historyView: "cards" });
    expect([s.theme, s.keepHistoryDays, s.historyView]).toEqual(["system", 90, "cards"]);
  });
});

describe("parseRecord", () => {
  const base = {
    id: 1,
    startedAt: 1_791_786_962,
    durationMs: 9000,
    kind: "offer",
    method: "icmp",
    medianMs: 24.4,
    p95Ms: null,
    jitterMs: 4.8,
    lossPct: 0,
    stalls: 0,
    bars: 5,
    ratePps: 52,
    seenBefore: 0,
    note: "",
    spark: [24, null, "x"],
    endpoint: null,
    protocol: "udp",
  };

  it("accepts a record and cleans the sparkline", () => {
    const r = parseRecord(base);
    expect(r.kind).toBe("offer");
    expect(r.spark).toEqual([24, null, null]);
  });

  it("reads the location and protocol", () => {
    const r = parseRecord({ ...base, endpoint: { kind: "peer", country: "BR" }, protocol: "tcp" });
    expect(r.endpoint).toEqual({ kind: "peer", country: "BR" });
    expect(r.protocol).toBe("tcp");
    expect(() => parseRecord({ ...base, protocol: "quic" })).toThrow();
  });

  it("rejects unknown kinds and methods", () => {
    expect(() => parseRecord({ ...base, kind: "draw" })).toThrow(/kind/);
    expect(() => parseRecord({ ...base, method: "telepathy" })).toThrow(/method/);
  });
});
