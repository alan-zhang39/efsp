import { cn } from "cn";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { endpointKey, endpointLabel } from "../lib/endpoint";
import { formatAge, formatCount, formatMs } from "../lib/format";
import {
  compareGamePings,
  compareWithPing,
  openTracesFolder,
  setRecording,
  testRouteProbe,
  devSeedHistory,
  type FlowView,
  type GamePings,
  type PingComparison,
  type ServerComparison,
  type Snapshot,
} from "../lib/monitor";
import type { NpcapStatus } from "../lib/npcap";
import { useMonitor } from "../lib/useMonitor";
import { MatchCard } from "./MatchCard";
import { NearestServers } from "./NearestServers";
import { Empty, InlineError, Page } from "./Page";
import { Status, type Tone } from "./Status";

/** A connection counts as new for this long, so it stands out when it appears. */
const NEW_FOR_MS = 5000;

interface Headline {
  tone: Tone;
  mark: string;
  label: string;
  heading: string;
  /** A line under the heading; null when the heading says it all. */
  body: string | null;
}

export function describe(snapshot: Snapshot | null, error: string | null): Headline {
  const off = (body: string): Headline => ({
    tone: "action",
    mark: "!",
    label: "Capture off",
    heading: "Live capture isn't running",
    body,
  });
  if (error !== null) return off(error);
  if (snapshot === null) {
    return {
      tone: "neutral",
      mark: "◌",
      label: "Starting",
      heading: "Starting capture",
      body: null,
    };
  }
  if (snapshot.source === "unavailable") return off(snapshot.error ?? "Capture is unavailable.");
  if (snapshot.game === "replayEnded") {
    return {
      tone: "neutral",
      mark: "◌",
      label: "Replay finished",
      heading: "End of the recorded trace",
      body: "Connections fade out as the replay clock runs on.",
    };
  }
  if (snapshot.game === "notRunning") {
    return {
      tone: "neutral",
      mark: "◌",
      label: "Waiting",
      heading: "Waiting for eFootball",
      body: null,
    };
  }
  return {
    tone: "neutral",
    mark: "◌",
    label: "Waiting",
    heading: "Waiting for a match",
    body: null,
  };
}

function FlowRow({ flow }: { flow: FlowView }) {
  const isNew = flow.ageMs < NEW_FOR_MS;
  const tag = flow.idle ? "Idle" : flow.sustained ? "Steady" : isNew ? "New" : null;
  return (
    // A quiet left edge marks the state; the text tag carries the meaning.
    <TableRow
      className="animate-fade shadow-[inset_2px_0_0_transparent] data-idle:text-muted-foreground data-new:bg-surface data-new:shadow-[inset_2px_0_0_var(--neutral)] data-sustained:shadow-[inset_2px_0_0_var(--ready)] motion-reduce:animate-none"
      data-new={isNew || undefined}
      data-sustained={flow.sustained || undefined}
      data-idle={flow.idle || undefined}
    >
      <th scope="row" className="px-3 py-2 text-left font-semibold whitespace-nowrap">
        Remote {flow.remoteLabel}
        {tag !== null && (
          <span
            className={cn(
              "ml-2 text-xs font-semibold",
              flow.sustained ? "text-ready" : "text-muted-foreground",
            )}
          >
            {tag}
          </span>
        )}
      </th>
      <TableCell className="py-2">{flow.protocol.toUpperCase()}</TableCell>
      <TableCell className="py-2">{endpointLabel(flow.endpoint)}</TableCell>
      <TableCell className="py-2 text-right">
        {flow.localPort} → {flow.remotePort}
      </TableCell>
      <TableCell className="py-2 text-right max-[640px]:hidden">{flow.ratePps}/s</TableCell>
      <TableCell className="py-2 text-right">
        {formatCount(flow.packetsIn)} / {formatCount(flow.packetsOut)}
      </TableCell>
      <TableCell className="py-2 text-right">{formatAge(flow.ageMs)}</TableCell>
    </TableRow>
  );
}

function FlowTable({ flows }: { flows: FlowView[] }) {
  if (flows.length === 0) {
    return <Empty>No connections yet.</Empty>;
  }
  return (
    <Table className="text-sm">
      <caption className="sr-only">eFootball connections, newest first</caption>
      <TableHeader>
        <TableRow>
          <TableHead scope="col">Remote</TableHead>
          <TableHead scope="col">Type</TableHead>
          <TableHead scope="col">Where</TableHead>
          <TableHead scope="col" className="text-right">
            Ports
          </TableHead>
          <TableHead scope="col" className="text-right max-[640px]:hidden">
            Rate
          </TableHead>
          <TableHead scope="col" className="text-right">
            In / out
          </TableHead>
          <TableHead scope="col" className="text-right">
            Age
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {flows.map((f) => (
          <FlowRow key={f.id} flow={f} />
        ))}
      </TableBody>
    </Table>
  );
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function describeComparison(c: PingComparison): string {
  if (c.efspMedianMs === null || c.pingExeMedianMs === null || c.differenceMs === null) {
    return `Not enough replies to compare (ping.exe got ${String(c.pingExeReplies)}).`;
  }
  const apart = `${formatMs(Math.abs(c.differenceMs))} apart`;
  return `efsp ${formatMs(c.efspMedianMs)}, ping.exe ${formatMs(c.pingExeMedianMs)} (${String(c.pingExeReplies)} replies): ${apart}, ${c.withinTolerance ? "within" : "outside"} ±5 ms.`;
}

function describeGamePings(g: GamePings | null): string | null {
  if (g === null) return null;
  const { nearestMs, typicalMs } = g;
  if (nearestMs === null || typicalMs === null) return null;
  return `Game's own server pings: ${String(g.servers)} servers, nearest ${formatMs(nearestMs)}, typical ${formatMs(typicalMs)}.`;
}

function describeServerRow(r: ServerComparison): string {
  const game = `game ${formatMs(r.gameMs)}`;
  if (r.icmpMs === null || r.differenceMs === null) {
    return `Server ${String(r.server)}: ${game}, no ICMP replies.`;
  }
  return `Server ${String(r.server)}: ${game}, ICMP ${formatMs(r.icmpMs)} (${String(r.icmpReplies)} replies), ${formatMs(Math.abs(r.differenceMs))} apart.`;
}

/** Dev builds only: trace recorder and the measurement cross-checks. */
function DevTools({ snapshot }: { snapshot: Snapshot }) {
  const { recording, measurement, gamePings } = snapshot;
  const [experimenting, setExperimenting] = useState(false);
  const [rows, setRows] = useState<ServerComparison[] | null>(null);
  const gamePingsNote = describeGamePings(gamePings);
  const [routeTest, setRouteTest] = useState<string | null>(null);
  const [seeded, setSeeded] = useState<string | null>(null);
  const [routeTesting, setRouteTesting] = useState(false);
  const testRoute = () => {
    setError(null);
    setRouteTest(null);
    setRouteTesting(true);
    testRouteProbe()
      .then(setRouteTest)
      .catch((e: unknown) => {
        setError(errorText(e));
      })
      .finally(() => {
        setRouteTesting(false);
      });
  };
  const [error, setError] = useState<string | null>(null);
  const [comparing, setComparing] = useState(false);
  const [comparison, setComparison] = useState<string | null>(null);
  const run = (action: Promise<void>) => {
    setError(null);
    action.catch((e: unknown) => {
      setError(errorText(e));
    });
  };
  const compare = () => {
    setError(null);
    setComparison(null);
    setComparing(true);
    compareWithPing()
      .then((c) => {
        setComparison(describeComparison(c));
      })
      .catch((e: unknown) => {
        setError(errorText(e));
      })
      .finally(() => {
        setComparing(false);
      });
  };
  const experiment = () => {
    setError(null);
    setRows(null);
    setExperimenting(true);
    compareGamePings()
      .then(setRows)
      .catch((e: unknown) => {
        setError(errorText(e));
      })
      .finally(() => {
        setExperimenting(false);
      });
  };
  return (
    <footer className="grid gap-2 border-t border-line pt-4" aria-label="Developer tools">
      <div className="flex flex-wrap gap-3">
        <Button
          className={cn(recording !== null && "border-action text-action")}
          onClick={() => {
            run(setRecording(recording === null));
          }}
        >
          {recording === null ? "Record trace" : "Stop recording"}
        </Button>
        <Button disabled={comparing || measurement?.method !== "icmp"} onClick={compare}>
          {comparing ? "Comparing for 10 s…" : "Compare with Windows ping"}
        </Button>
        <Button
          disabled={experimenting || measurement !== null || gamePings?.servers === 0}
          onClick={experiment}
        >
          {experimenting ? "Comparing game pings…" : "Compare game pings with ICMP"}
        </Button>
        <Button disabled={routeTesting || measurement !== null} onClick={testRoute}>
          {routeTesting ? "Testing…" : "Test route probing"}
        </Button>
        <Button
          variant="quiet"
          onClick={() => {
            run(openTracesFolder());
          }}
        >
          Open traces folder
        </Button>
        <Button
          variant="quiet"
          onClick={() => {
            setError(null);
            devSeedHistory().then(
              (n) => {
                setSeeded(`Added ${String(n)} sample matches to history.`);
              },
              (e: unknown) => {
                setError(errorText(e));
              },
            );
          }}
        >
          Add sample matches
        </Button>
      </div>
      <p className="m-0 text-[0.8125rem] text-muted-foreground">
        {recording === null
          ? "Traces keep packet headers and timing only, with every address replaced."
          : `Recording ${recording.fileName}, ${formatCount(recording.packets)} packets`}
      </p>
      {comparison !== null && (
        <p className="m-0 text-[0.8125rem] text-muted-foreground">{comparison}</p>
      )}
      {gamePingsNote !== null && (
        <p className="m-0 text-[0.8125rem] text-muted-foreground">{gamePingsNote}</p>
      )}
      {snapshot.tcpMatches !== null && snapshot.tcpMatches > 0 && (
        <p className="m-0 text-[0.8125rem] text-muted-foreground">{`Possible TCP matches this run: ${String(snapshot.tcpMatches)}.`}</p>
      )}
      {gamePings !== null && gamePings.regions.length > 0 && (
        <ul
          className="m-0 pl-4 text-[0.8125rem] text-muted-foreground"
          aria-label="Game's pings by region"
        >
          {gamePings.regions.map((r) => (
            <li key={endpointKey(r.endpoint)}>
              {`${endpointLabel(r.endpoint)}: ${formatMs(r.bestMs)} (${String(r.servers)} ${r.servers === 1 ? "server" : "servers"})`}
            </li>
          ))}
        </ul>
      )}
      {routeTest !== null && (
        <p className="m-0 text-[0.8125rem] text-muted-foreground">{routeTest}</p>
      )}
      {seeded !== null && <p className="m-0 text-[0.8125rem] text-muted-foreground">{seeded}</p>}
      {rows !== null && (
        <ul className="m-0 pl-4 text-[0.8125rem] text-muted-foreground">
          {rows.map((r) => (
            <li key={r.server}>{describeServerRow(r)}</li>
          ))}
        </ul>
      )}
      {error !== null && <InlineError>{error}</InlineError>}
    </footer>
  );
}

/**
 * One polite screen-reader announcement per new match, once its first
 * numbers are in. The card itself isn't a live region: it updates 4×/s.
 */
function useMatchAnnouncement(live: Snapshot["measurement"]): string {
  const [said, setSaid] = useState<{ flowId: number; text: string } | null>(null);
  const flowId = live?.flowId ?? null;
  const median = live?.ping?.medianMs ?? null;
  const bars = live?.bars ?? null;
  // React's "adjust state during render" pattern: when a new match first has
  // numbers, remember the sentence for it. Later ticks for the same match
  // leave it alone, so it's announced once.
  if (flowId !== null && said?.flowId !== flowId && median !== null && bars !== null) {
    setSaid({ flowId, text: `Match found: ${formatMs(median)}, ${String(bars)} of 5 bars.` });
  }
  return said?.text ?? "";
}

export function MonitorScreen({
  npcap,
  onOpenServers,
}: {
  npcap: NpcapStatus;
  onOpenServers?: () => void;
}) {
  const { snapshot, error } = useMonitor();
  const announcement = useMatchAnnouncement(snapshot?.measurement ?? null);
  const headline = describe(snapshot, error);
  const source =
    snapshot?.source === "replay"
      ? "Replaying a trace"
      : snapshot?.source === "live"
        ? "Live capture"
        : null;
  const driver =
    npcap.state === "found" && npcap.version !== null ? `Npcap ${npcap.version}` : null;
  const live = snapshot?.measurement ?? null;

  return (
    <Page className="grid-rows-[auto_1fr_auto]">
      {/* Capture details are for developers; players just see the card. */}
      {snapshot?.devTools === true && (
        <header className="flex justify-end">
          <p className="m-0 flex gap-4 text-[0.8125rem] text-muted-foreground">
            {[source, driver]
              .filter((x) => x !== null)
              .map((x) => (
                <span key={x}>{x}</span>
              ))}
          </p>
        </header>
      )}

      <p className="sr-only" aria-live="polite">
        {announcement}
      </p>

      <div className="grid content-start gap-8 *:min-w-0">
        {/* Release builds show nothing about connections until there is a
            match connection: the live card during a match, and the
            status (like every other status) before and after one. The last
            match is in History. */}
        {live !== null && snapshot !== null ? (
          <MatchCard m={live} mode="live" />
        ) : (
          <div aria-live="polite">
            <Status
              key={headline.heading}
              tone={headline.tone}
              mark={headline.mark}
              label={headline.label}
              heading={headline.heading}
              body={headline.body}
            />
            {snapshot?.game === "running" && snapshot.source === "live" && (
              <NearestServers
                regions={snapshot.gamePings?.regions ?? []}
                onOpenServers={onOpenServers}
              />
            )}
          </div>
        )}

        {snapshot !== null && snapshot.source !== "unavailable" && snapshot.error !== null && (
          <InlineError>{snapshot.error}</InlineError>
        )}

        {snapshot?.devTools && snapshot.source !== "unavailable" && (
          <section
            className="grid gap-3 border-t border-dashed border-line pt-4"
            aria-label="Developer view"
          >
            <p className="m-0 text-[0.8125rem] font-semibold text-muted-foreground">
              Developer view
            </p>
            <FlowTable flows={snapshot.flows} />
          </section>
        )}
      </div>

      {snapshot?.source === "live" && snapshot.devTools && <DevTools snapshot={snapshot} />}
    </Page>
  );
}
