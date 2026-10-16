import { cn } from "cn";
import { Tooltip } from "@/components/ui/tooltip";
import { useEffect, useId, useRef, useState } from "react";
import { formatMs } from "../lib/format";
import uPlot from "uplot";
import type { Point } from "../lib/monitor";

/** The zoom hint and reset share one small framed box at the top right. */
const ZOOM =
  "absolute top-0 right-0 z-[2] inline-flex items-center rounded-md border border-line bg-background text-muted-foreground";

/** Live graphs show this much. */
const LIVE_SECONDS = 60;
/** The ping graph is at least this tall... */
const PING_HEIGHT = 180;
/** ...and grows into the window's free space up to this. */
const PING_MAX_HEIGHT = 560;
/** Room kept under the graph (page padding, and the packets strip if shown). */
const BELOW = 40;
const PACKETS_HEIGHT = 64;

export interface GraphShow {
  jitter: boolean;
  losses: boolean;
  packets: boolean;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** "#e4e8ec" + 0.14 -> "rgba(228, 232, 236, 0.14)". Canvas needs a plain colour. */
function withAlpha(hex: string, alpha: number): string {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (m === null) return `rgba(128, 128, 128, ${String(alpha)})`;
  const [r, g, b] = [m[1], m[2], m[3]].map((h) => parseInt(h ?? "80", 16));
  return `rgba(${String(r)}, ${String(g)}, ${String(b)}, ${String(alpha)})`;
}

/**
 * Ping chart columns: x, ping, jitter low/high, loss marks. Ping gaps (ticks
 * with no new reply) are bridged; the band follows the last known ping and
 * exists only where there is probe jitter.
 */
function pingColumns(points: Point[]): uPlot.AlignedData {
  const x: number[] = [];
  const rtt: (number | null)[] = [];
  const low: (number | null)[] = [];
  const high: (number | null)[] = [];
  const lost: (number | null)[] = [];
  let lastRtt: number | null = null;
  for (const p of points) {
    x.push(p.tMs / 1000);
    rtt.push(p.rttMs);
    if (p.rttMs !== null) lastRtt = p.rttMs;
    const j = p.jitterMs;
    low.push(lastRtt === null || j === null ? null : Math.max(0, lastRtt - j));
    high.push(lastRtt === null || j === null ? null : lastRtt + j);
    // Lost probes and stalls: a mark just above the ping line.
    lost.push(p.lost > 0 || p.stalls > 0 ? (lastRtt ?? 0) + (j ?? 0) + 6 : null);
  }
  return [x, rtt, low, high, lost];
}

function packetColumns(points: Point[]): uPlot.AlignedData {
  return [points.map((p) => p.tMs / 1000), points.map((p) => p.ratePps)];
}

const secondsAxis = (muted: string, line: string): uPlot.Axis => ({
  stroke: muted,
  grid: { stroke: line, width: 1 },
  ticks: { show: false },
  values: (_u, splits) => splits.map((s) => `${String(Math.round(s))} s`),
});

/** Fills the window below `el`, between the minimum and maximum height. */
function fillHeight(el: HTMLElement, reserve: number): number {
  const free = window.innerHeight - el.getBoundingClientRect().top - reserve;
  return Math.round(Math.min(PING_MAX_HEIGHT, Math.max(PING_HEIGHT, free)));
}

/**
 * Creates a uPlot chart in `el` that follows its width, and its height too
 * when `height` is a function (re-measured when the window resizes).
 */
function mount(
  el: HTMLDivElement,
  opts: Omit<uPlot.Options, "width" | "height"> & { height: number | (() => number) },
  data: uPlot.AlignedData,
): { chart: uPlot; dispose: () => void } {
  const height = () => (typeof opts.height === "function" ? opts.height() : opts.height);
  const chart = new uPlot({ ...opts, width: el.clientWidth, height: height() }, data, el);
  const resize = () => {
    chart.setSize({ width: el.clientWidth, height: height() });
  };
  // Follow the graph's own width, and the window's size: the app frame is
  // sized to the window, so it changes whenever the window does (more
  // reliably than the window "resize" event in every webview).
  const observer = new ResizeObserver(resize);
  observer.observe(el);
  const frame = document.querySelector("[data-app-frame]");
  if (frame !== null) observer.observe(frame);
  window.addEventListener("resize", resize);
  return {
    chart,
    dispose: () => {
      observer.disconnect();
      window.removeEventListener("resize", resize);
      chart.destroy();
    },
  };
}

interface Props {
  points: Point[];
  /** Live: follow the last 60 s. Otherwise: whole match, drag to zoom. */
  live: boolean;
  show: GraphShow;
  /** Redraw when the theme changes (colours are read at creation). */
  theme: string;
}

/**
 * Ping over time, one y-axis (ms). Packets/s, a different unit, gets its own
 * thin strip below instead of a second axis on the same chart.
 */
interface Hover {
  left: number;
  top: number;
  ms: number | null;
  /** Seconds into the match at the cursor. */
  at: number;
}

/**
 * The ping at data index `idx`: that point's reply, or the nearest reply
 * within two points either side (replies don't land on every 250 ms tick).
 */
function pingNear(u: uPlot, idx: number): { ms: number; idx: number } | null {
  const rtt = u.data[1];
  if (rtt === undefined) return null;
  for (const d of [0, -1, 1, -2, 2]) {
    const v = rtt[idx + d];
    if (typeof v === "number") return { ms: v, idx: idx + d };
  }
  return null;
}

function ZoomIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <circle cx="8.5" cy="8.5" r="5.5" />
      <path d="M12.5 12.5l4.5 4.5M6.5 8.5h4M8.5 6.5v4" />
    </svg>
  );
}

export function PingGraph({ points, live, show, theme }: Props) {
  // React's state setter never changes identity, so uPlot's hook (created
  // once per chart) can call it directly.
  const [hover, setHover] = useState<Hover | null>(null);
  // Zoomed in after the match (drag across the graph): offer a way back.
  const [zoomed, setZoomed] = useState(false);
  const pingHost = useRef<HTMLDivElement>(null);
  const packetsHost = useRef<HTMLDivElement>(null);
  const pingPlot = useRef<uPlot | null>(null);
  const packetsPlot = useRef<uPlot | null>(null);
  // Links hover and drag-zoom between the ping chart and the packets strip.
  const syncKey = useId();

  // Create once per mode / visible series / theme; data updates below don't
  // rebuild, so ticks don't animate or flash.
  useEffect(() => {
    const el = pingHost.current;
    if (el === null) return;
    const text = cssVar("--text");
    const muted = cssVar("--text-muted");
    const line = cssVar("--line");
    const mark = cssVar("--neutral");
    const { chart, dispose } = mount(
      el,
      {
        // Fill the space below; leave room for the packets strip if shown.
        height: () => fillHeight(el, BELOW + (show.packets ? PACKETS_HEIGHT + 16 : 0)),
        legend: { show: false },
        // Drag to zoom (after the match). The selection overlay is styled in
        // CSS; uPlot clears it once the zoom is applied.
        cursor: { drag: { x: !live, y: false, setScale: true }, sync: { key: syncKey } },
        hooks: {
          // Zoomed means the x range is narrower than the whole match.
          setScale: [
            (u, key) => {
              if (key !== "x" || live) return;
              const xs = u.data[0];
              const first = xs[0] ?? 0;
              const last = xs[xs.length - 1] ?? 0;
              const { min, max } = u.scales.x ?? {};
              setZoomed(
                min !== undefined && max !== undefined && (min > first + 0.5 || max < last - 0.5),
              );
            },
          ],
          // Show the actual ping where the cursor is.
          setCursor: [
            (u) => {
              const idx = u.cursor.idx;
              const left = u.cursor.left ?? -1;
              if (idx === null || idx === undefined || left < 0) {
                setHover(null);
                return;
              }
              const near = pingNear(u, idx);
              const at = u.data[0][idx] ?? 0;
              // Position relative to the chart host, above the line.
              const overLeft = u.over.offsetLeft;
              const overTop = u.over.offsetTop;
              const y = near === null ? (u.cursor.top ?? 0) : u.valToPos(near.ms, "y");
              setHover({
                left: overLeft + left,
                top: overTop + y,
                ms: near?.ms ?? null,
                at,
              });
            },
          ],
        },
        scales: {
          x: { time: false },
          y: { range: (_u, _min, max) => [0, Math.max(60, Math.ceil((max + 10) / 20) * 20)] },
        },
        axes: [
          secondsAxis(muted, line),
          {
            stroke: muted,
            grid: { stroke: line, width: 1 },
            ticks: { show: false },
            values: (_u, splits) => splits.map((s) => `${String(s)} ms`),
            size: 56,
            // A tall graph still gets only a few calm gridlines.
            space: 56,
          },
        ],
        series: [
          {},
          { label: "Ping", stroke: text, width: 2, spanGaps: true, points: { show: false } },
          {
            label: "Jitter low",
            show: show.jitter,
            stroke: "transparent",
            points: { show: false },
          },
          {
            label: "Jitter high",
            show: show.jitter,
            stroke: "transparent",
            points: { show: false },
          },
          {
            label: "Lost ping or stall",
            show: show.losses,
            stroke: "transparent",
            // Neutral, so it isn't confused with the 3-bar amber.
            points: { show: true, size: 8, fill: mark, stroke: mark },
          },
        ],
        bands: show.jitter ? [{ series: [3, 2], fill: withAlpha(text, 0.14) }] : [],
      },
      pingColumns([]),
    );
    pingPlot.current = chart;
    return () => {
      dispose();
      pingPlot.current = null;
    };
  }, [live, show.jitter, show.losses, show.packets, theme, syncKey]);

  useEffect(() => {
    const el = packetsHost.current;
    if (el === null || !show.packets) return;
    const muted = cssVar("--text-muted");
    const line = cssVar("--line");
    const { chart, dispose } = mount(
      el,
      {
        height: PACKETS_HEIGHT,
        legend: { show: false },
        cursor: { drag: { x: false, y: false }, sync: { key: syncKey } },
        scales: {
          x: { time: false },
          y: { range: (_u, _min, max) => [0, Math.max(10, max * 1.2)] },
        },
        axes: [
          { ...secondsAxis(muted, line), show: false },
          {
            stroke: muted,
            grid: { stroke: line, width: 1 },
            ticks: { show: false },
            values: (_u, splits) => splits.map((s) => `${String(Math.round(s))}/s`),
            size: 56,
            splits: (_u, _ax, _min, max) => [0, Math.round(max / 2)],
          },
        ],
        series: [{}, { label: "Packets in/s", stroke: muted, width: 1.5, points: { show: false } }],
      },
      packetColumns([]),
    );
    packetsPlot.current = chart;
    return () => {
      dispose();
      packetsPlot.current = null;
    };
  }, [show.packets, theme, syncKey]);

  useEffect(() => {
    const ping = pingPlot.current;
    const end = Math.max(LIVE_SECONDS, (points.at(-1)?.tMs ?? 0) / 1000);
    if (ping !== null) {
      ping.setData(pingColumns(points), !live);
      if (live) ping.setScale("x", { min: end - LIVE_SECONDS, max: end });
    }
    const packets = packetsPlot.current;
    if (packets !== null) {
      packets.setData(packetColumns(points), !live);
      if (live) packets.setScale("x", { min: end - LIVE_SECONDS, max: end });
    }
  }, [points, live, show.jitter, show.losses, show.packets, theme]);

  const resetZoom = () => {
    const u = pingPlot.current;
    if (u === null) return;
    const xs = u.data[0];
    u.setScale("x", { min: xs[0] ?? 0, max: xs[xs.length - 1] ?? 0 });
  };

  return (
    <figure
      className="ping-graph m-0 grid min-w-0 gap-2"
      data-live={live || undefined}
      aria-label={live ? "Ping over the last minute" : "Ping over the whole match"}
    >
      <div
        ref={pingHost}
        className={cn("relative min-h-[180px] w-full min-w-0", !live && "cursor-crosshair")}
      >
        {/* After a match: a quiet hint that the graph zooms, and a way back. */}
        {!live &&
          (zoomed ? (
            <button
              type="button"
              className={cn(
                ZOOM,
                "cursor-pointer px-2.5 py-1 text-[0.8125rem] text-foreground hover:bg-surface focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
              )}
              onClick={resetZoom}
            >
              Reset zoom
            </button>
          ) : (
            <Tooltip label="Drag across the graph to zoom" side="left">
              <span
                className={cn(ZOOM, "size-7 cursor-help justify-center")}
                aria-label="Drag across the graph to zoom"
                role="img"
              >
                <ZoomIcon />
              </span>
            </Tooltip>
          ))}
        {hover !== null && (
          <div
            className="pointer-events-none absolute z-[2] -translate-x-1/2 -translate-y-[calc(100%+12px)] rounded-md border border-line bg-surface px-2 py-1 text-[0.8125rem] whitespace-nowrap text-foreground [&_strong]:font-[650]"
            style={{ left: `${String(hover.left)}px`, top: `${String(hover.top)}px` }}
          >
            {hover.ms === null ? (
              "No ping reply here"
            ) : (
              <>
                <strong>{formatMs(hover.ms)}</strong> at {Math.round(hover.at)} s
              </>
            )}
          </div>
        )}
      </div>
      {show.packets && (
        <div className="grid grid-cols-[auto_1fr] items-center gap-2">
          <span className="rotate-180 text-xs text-muted-foreground [writing-mode:vertical-rl]">
            Packets in/s
          </span>
          <div ref={packetsHost} />
        </div>
      )}
    </figure>
  );
}
