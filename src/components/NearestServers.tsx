import { useState } from "react";
import { endpointKey, endpointLabel } from "../lib/endpoint";
import type { RegionPing } from "../lib/monitor";
import { Button } from "@/components/ui/button";
import { PingBars } from "./BarsGlyph";

/** How many cities to show while waiting. */
const SHOWN = 5;

interface City {
  key: string;
  name: string;
  ms: number;
  bars: number;
}

/** One row per city (the game may ping several servers in one), fastest first. */
function cities(regions: RegionPing[]): City[] {
  const byKey = new Map<string, City>();
  for (const r of regions) {
    if (r.endpoint.kind !== "server") continue;
    const key = endpointKey(r.endpoint);
    const seen = byKey.get(key);
    if (seen !== undefined && seen.ms <= r.bestMs) continue;
    byKey.set(key, { key, name: endpointLabel(r.endpoint), ms: r.bestMs, bars: r.bars });
  }
  return [...byKey.values()].sort((a, b) => a.ms - b.ms);
}

/** How far apart two shown cities must be before they swap places. */
const SWAP_MS = 5;

/**
 * Whether the shown cities should be picked again: a different city is now
 * among the nearest, or two shown cities are clearly in the wrong order.
 * Small ping changes leave the rows where they are, so they don't jump.
 */
function stale(order: string[], now: City[]): boolean {
  const top = now.slice(0, SHOWN).map((c) => c.key);
  if (top.length !== order.length || top.some((k) => !order.includes(k))) return true;
  const ms = new Map(now.map((c) => [c.key, c.ms]));
  return order.some((k, i) => {
    const next = order[i + 1];
    return next !== undefined && (ms.get(k) ?? 0) - (ms.get(next) ?? 0) > SWAP_MS;
  });
}

/**
 * While waiting for a match: the game's own pings to its nearest cities, so
 * the wait shows where a match is likely to land. The rows follow the
 * game's pings, but only move when the nearest cities change or the order
 * is clearly off.
 */
export function NearestServers({
  regions,
  onOpenServers,
}: {
  regions: RegionPing[];
  onOpenServers?: (() => void) | undefined;
}) {
  const now = cities(regions);
  const [order, setOrder] = useState<string[]>([]);
  // Re-pick when the nearest cities change (adjusting state during render,
  // React's pattern for state derived from changing input).
  if (stale(order, now)) {
    setOrder(now.slice(0, SHOWN).map((c) => c.key));
  }
  const byKey = new Map(now.map((c) => [c.key, c]));
  const rows = order.flatMap((k) => byKey.get(k) ?? []);
  if (rows.length === 0) return null;

  return (
    <section
      className="mt-[clamp(2rem,6vh,4rem)] max-w-[clamp(22rem,38vw,34rem)] text-[clamp(0.9375rem,1.3vw,1.125rem)]"
      aria-labelledby="nearest-title"
    >
      <h2
        id="nearest-title"
        className="m-0 mb-2 text-[0.8125em] font-semibold text-muted-foreground"
      >
        Nearest servers
      </h2>
      <ul className="m-0 list-none p-0">
        {rows.map((c) => (
          <li
            key={c.key}
            className="flex items-center justify-between gap-4 py-[0.7em] [&+&]:border-t [&+&]:border-line"
          >
            <span className="font-semibold">{c.name}</span>
            <PingBars ms={c.ms} bars={c.bars} glyphClassName="h-[1.4em] w-[1.75em]" />
          </li>
        ))}
      </ul>
      {onOpenServers !== undefined && (
        <Button variant="text" className="mt-3 text-[0.875em]" onClick={onOpenServers}>
          All servers
        </Button>
      )}
    </section>
  );
}
