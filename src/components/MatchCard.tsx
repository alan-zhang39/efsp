import { cn } from "cn";
import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { tierFor } from "../lib/bars";
import { endpointLabel } from "../lib/endpoint";
import { formatMs } from "../lib/format";
import type { Measurement as M, Point } from "../lib/monitor";
import type { LiveLayout } from "../lib/settings";
import { useSettings } from "../lib/SettingsContext";
import { BarsGlyph, TIER_TEXT } from "./BarsGlyph";
import { PingGraph } from "./PingGraph";

interface Props {
  m: M;
  /** "live" follows the last 60 s; otherwise the whole timeline is shown. */
  mode: "live" | "saved";
  /** Whole timeline, when not live (live uses m.recent). */
  timeline?: Point[];
  /** "card": details beside the ping (L1). "focus": one big centred ping (L2). */
  layout?: LiveLayout;
  /** Top-right details, e.g. a saved match's date and time. */
  meta?: ReactNode;
}

/** "4:12" for a live match's running time. */
function clock(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${String(Math.floor(s / 60))}:${String(s % 60).padStart(2, "0")}`;
}

/** The big number: ping, "…" while still looking, "—" without one. */
function pingText(m: M): string {
  const median = m.ping?.medianMs ?? null;
  if (median !== null) return formatMs(median);
  return m.method === "passive" ? "—" : "…";
}

/**
 * The ping's size: largest while live, a step smaller for a saved match;
 * the focus layout (L2) is bigger again. All grow with the window.
 */
function pingSize(live: boolean, focus: boolean): string {
  if (live) return focus ? "text-[clamp(4rem,14vw,7rem)]" : "text-[clamp(3rem,9vw,4.5rem)]";
  return focus ? "text-[clamp(3rem,10vw,5rem)]" : "text-[clamp(2.5rem,7vw,3.75rem)]";
}

/**
 * The product: where, the ping and bars, then jitter and loss and the
 * graph. No explanations; the numbers speak.
 */
export function MatchCard({ m, mode, timeline, layout = "card", meta }: Props) {
  const { settings, resolvedTheme } = useSettings();
  const { show } = settings;
  const live = mode === "live";
  const focus = layout === "focus";
  const place = m.endpoint === null ? "Match" : endpointLabel(m.endpoint);
  const median = m.ping?.medianMs ?? null;
  const approx = m.method === "routeEstimate" && median !== null;

  const stats: { label: string; value: string }[] = [];
  if (m.ping !== null && show.jitter)
    stats.push({ label: "Jitter", value: formatMs(m.ping.jitterMs) });
  if (m.ping !== null && show.loss)
    stats.push({ label: "Loss", value: `${String(m.ping.lossPct)}%` });
  if (show.packets) {
    stats.push({ label: "Packets/s", value: String(Math.round(m.passive.ratePps)) });
  }

  const Heading = live ? "h1" : "h2";
  const placeLine = (
    <Heading
      className={cn(
        "m-0 text-base font-semibold",
        focus ? "font-medium text-muted-foreground" : "mb-2",
      )}
    >
      {place}
      {m.protocol === "tcp" && <Badge>TCP</Badge>}
    </Heading>
  );
  const hero = (
    <div className="flex items-end gap-4">
      <p
        className={cn(
          // Size first: cn drops a line height that comes before a font size.
          pingSize(live, focus),
          "m-0 leading-[0.95] font-[650] tracking-[-0.035em] tabular-nums",
          median === null || m.bars === null
            ? median === null && "text-muted-foreground"
            : TIER_TEXT[tierFor(m.bars)],
        )}
      >
        {approx && <span className="mr-[0.08em] font-medium text-muted-foreground">≈</span>}
        {pingText(m)}
      </p>
      {m.bars !== null && (
        <BarsGlyph
          bars={m.bars}
          className={cn("mb-[0.35rem]", focus ? "h-[4.4rem] w-[5.5rem]" : "h-[3.6rem] w-[4.5rem]")}
        />
      )}
    </div>
  );
  const status =
    meta ??
    (live && (
      <p className="m-0 inline-flex items-center gap-1.5 text-sm text-muted-foreground tabular-nums">
        <span className="size-2 rounded-full bg-ready" aria-hidden="true" />
        Live · {clock(m.durationMs)}
      </p>
    ));

  return (
    <section
      className="grid animate-settle gap-4 *:min-w-0 motion-reduce:animate-fade"
      data-layout={layout}
      data-mode={mode}
      data-tier={m.bars === null ? undefined : tierFor(m.bars)}
      aria-label={live ? "Match connection" : "Match"}
    >
      {focus ? (
        <div className="grid justify-items-center gap-2 pt-4 text-center">
          <div>{status}</div>
          {placeLine}
          {hero}
          {stats.length > 0 && (
            <p className="m-0 flex gap-6 text-[0.9375rem] text-muted-foreground">
              {stats.map((s) => (
                <span key={s.label}>
                  {s.label}{" "}
                  <strong className="font-semibold text-foreground tabular-nums">{s.value}</strong>
                </span>
              ))}
            </p>
          )}
        </div>
      ) : (
        <>
          <div className="flex items-start justify-between gap-4">
            <div>
              {placeLine}
              {hero}
            </div>
            <div className="text-right">{status}</div>
          </div>
          {stats.length > 0 && (
            <dl className="m-0 flex gap-6">
              {stats.map((s) => (
                <div key={s.label} className="grid gap-0.5">
                  <dt className="text-[0.8125rem] text-muted-foreground">{s.label}</dt>
                  <dd className="m-0 text-[1.0625rem] font-semibold tabular-nums">{s.value}</dd>
                </div>
              ))}
            </dl>
          )}
        </>
      )}

      <PingGraph
        points={live ? m.recent : (timeline ?? [])}
        live={live}
        show={{ jitter: show.graphJitter, losses: show.graphLosses, packets: show.graphPackets }}
        theme={resolvedTheme}
      />
    </section>
  );
}
