import {
  createColumnHelper,
  getCoreRowModel,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import { cn } from "cn";
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tooltip } from "@/components/ui/tooltip";
import { type Continent, type Endpoint, endpointKey } from "../lib/endpoint";
import { useLatestSnapshot } from "../lib/liveStore";
import {
  type Area,
  applyPicker,
  chooseGameExe,
  type PickerStatus,
  refreshPicker,
  usePicker,
} from "../lib/picker";
import type { HistoryView as View } from "../lib/settings";
import { useSettings } from "../lib/SettingsContext";
import { PingBars } from "./BarsGlyph";
import { DataTable } from "./DataTable";
import { RefreshIcon } from "./icons";
import { Empty, FilterBar, Hint, InlineError, Page, PageHeader } from "./Page";
import { ViewToggle } from "./ViewToggle";

type Server = Extract<Endpoint, { kind: "server" }>;

/** One city on the page and the game's live ping to it. */
interface Region {
  key: string;
  name: string;
  continent: Continent | null;
  /** The game's own ping (fastest server there); null if it doesn't ping it now. */
  pingMs: number | null;
  bars: number | null;
}

const CONTINENTS: Record<Continent, string> = {
  europe: "Europe",
  northAmerica: "North America",
  southAmerica: "South America",
  asia: "Asia",
  oceania: "Oceania",
  africa: "Africa",
  middleEast: "Middle East",
};

const MIN_BARS: { label: string; bars: number | null }[] = [
  { label: "Any bars", bars: null },
  { label: "5 bars", bars: 5 },
  { label: "4 bars or more", bars: 4 },
  { label: "3 bars or more", bars: 3 },
  { label: "2 bars or more", bars: 2 },
];

const MAX_PING: { label: string; ms: number | null }[] = [
  { label: "Any ping", ms: null },
  { label: "Up to 50 ms", ms: 50 },
  { label: "Up to 80 ms", ms: 80 },
  { label: "Up to 120 ms", ms: 120 },
];

/** The city; a region without a published city goes by its code. */
function regionName(e: Server): string {
  return e.place === null ? e.region : e.place.city;
}

/**
 * The game's live pings per city (endpointKey): players want "Kuala Lumpur",
 * not a region code. Switched-off cities stay
 * listed (the game stops pinging them) so they can be switched back on.
 */
function mergeRegions(
  live: { endpoint: Endpoint; bestMs: number; bars: number }[],
  off: Area[],
  /** The last ping seen for each city, for ones switched off since. */
  known: Map<string, Region>,
): Region[] {
  const byKey = new Map<string, Region>();
  for (const r of live) {
    if (r.endpoint.kind !== "server") continue;
    const key = endpointKey(r.endpoint);
    if (byKey.has(key)) continue;
    byKey.set(key, {
      key,
      name: regionName(r.endpoint),
      continent: r.endpoint.place?.continent ?? null,
      pingMs: r.bestMs,
      bars: r.bars,
    });
  }
  // A switched-off city isn't pinged any more: keep it where it was, with
  // the ping it had before (or just its name if it was never seen).
  for (const a of off) {
    if (!byKey.has(a.key)) {
      byKey.set(
        a.key,
        known.get(a.key) ?? { key: a.key, name: a.name, continent: null, pingMs: null, bars: null },
      );
    }
  }
  return [...byKey.values()];
}

/**
 * Sorts by name or ping. Servers without a ping (not pinged now) go last
 * whichever way, in name order, so the list stays put.
 */
function sortRegions(regions: Region[], by: { id: string; desc: boolean }): Region[] {
  const dir = by.desc ? -1 : 1;
  const byName = (a: Region, b: Region) => a.name.localeCompare(b.name);
  return [...regions].sort((a, b) => {
    if (by.id === "name") return dir * byName(a, b);
    if (a.pingMs === null || b.pingMs === null) {
      if (a.pingMs === b.pingMs) return byName(a, b);
      return a.pingMs === null ? 1 : -1;
    }
    return dir * (a.pingMs - b.pingMs) || byName(a, b);
  });
}

function PingCell({ r, className }: { r: Region; className?: string }) {
  if (r.pingMs === null) {
    return <span className="text-sm text-muted-foreground">Not pinged now</span>;
  }
  return <PingBars ms={r.pingMs} bars={r.bars} className={className} />;
}

/** Switch state for the region picker; null when the picker isn't available. */
export interface PickerSwitches {
  isOn: (r: Region) => boolean;
  set: (r: Region, on: boolean) => void;
  /** Every listed city is on. */
  allOn: boolean;
  setAll: (on: boolean) => void;
}

/** On = matches may land here. A switch, so on/off reads at a glance. */
function OnSwitch({ r, toggle }: { r: Region; toggle: PickerSwitches }) {
  return (
    <Switch
      aria-label={`Allow matches in ${r.name}`}
      checked={toggle.isOn(r)}
      onCheckedChange={(on) => {
        toggle.set(r, on);
      }}
    />
  );
}

function NameCell({ r, toggle }: { r: Region; toggle: PickerSwitches | null }) {
  const off = toggle !== null && !toggle.isOn(r);
  return (
    <span className={cn("block font-semibold", off && "text-muted-foreground line-through")}>
      {r.name}
      {off && <span className="sr-only"> (off)</span>}
    </span>
  );
}

const col = createColumnHelper<Region>();

/**
 * The table's columns. Defined once: TanStack renders each `cell` as a
 * component, so new functions every render would remount the cells (and
 * drop focus and clicks). The changing picker state comes in through
 * the table's `meta` instead.
 */
const COLUMNS = [
  col.accessor("name", {
    header: "Server",
    cell: (c) => (
      <NameCell r={c.row.original} toggle={c.table.options.meta?.serverToggle ?? null} />
    ),
    meta: { rowHeader: true },
  }),
  col.accessor("pingMs", {
    id: "ping",
    header: "Ping",
    cell: (c) => <PingCell r={c.row.original} />,
  }),
  col.display({
    id: "on",
    header: (c) => {
      const toggle = c.table.options.meta?.serverToggle;
      return toggle == null ? null : (
        <Tooltip label={toggle.allOn ? "Switch all off" : "Switch all on"}>
          <Switch aria-label="All servers" checked={toggle.allOn} onCheckedChange={toggle.setAll} />
        </Tooltip>
      );
    },
    cell: (c) => {
      const toggle = c.table.options.meta?.serverToggle;
      return toggle == null ? null : <OnSwitch r={c.row.original} toggle={toggle} />;
    },
    meta: { className: "w-12 text-right" },
  }),
];

function Cards({ regions, toggle }: { regions: Region[]; toggle: PickerSwitches | null }) {
  return (
    <ul className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3 p-0">
      {regions.map((r) => (
        <li key={r.key} className="grid gap-1 rounded-[0.625rem] border border-line p-4">
          <span className="flex items-center justify-between gap-2">
            <NameCell r={r} toggle={toggle} />
            {toggle !== null && <OnSwitch r={r} toggle={toggle} />}
          </span>
          <PingCell
            r={r}
            className="mt-2 text-[1.75rem] leading-none font-[650] tracking-[-0.02em]"
          />
        </li>
      ))}
    </ul>
  );
}

/**
 * The region picker's controls: Apply when something changed, and
 * Find eFootball.exe when Steam doesn't know where it is. Turning every
 * city back on is the "All servers" switch, then Apply.
 */
function PickerPanel({
  status,
  dirty,
  blocked,
  apply,
  reset,
}: {
  status: PickerStatus;
  dirty: boolean;
  /** Why Apply can't run now, if it can't. */
  blocked: string | null;
  /** Resolves false if the player cancelled. */
  apply: () => Promise<boolean>;
  /** Drops unapplied changes: back to what's in effect. */
  reset: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const run = (action: () => Promise<boolean>) => {
    setBusy(true);
    setMessage(null);
    action()
      .catch((e: unknown) => {
        setMessage(e instanceof Error ? e.message : String(e));
        // A failed change may still have changed something: re-read it.
        void refreshPicker().catch(() => undefined);
        return false;
      })
      .finally(() => {
        setBusy(false);
      });
  };

  // Steam's records nearly always find the game; ask only when they didn't
  // and the player is actually switching cities off.
  const findExe = status.gameExe === null && dirty;
  if (!status.available || (!dirty && !findExe && message === null)) return null;
  return (
    <div className="grid justify-items-end gap-2">
      <div className="flex flex-wrap gap-2">
        {findExe && (
          <Button
            variant="quiet"
            disabled={busy}
            onClick={() => {
              run(chooseGameExe);
            }}
          >
            Find eFootball.exe…
          </Button>
        )}
        {dirty && (
          <Button variant="quiet" disabled={busy} onClick={reset}>
            Reset
          </Button>
        )}
        {dirty && (
          <Button
            disabled={busy || blocked !== null}
            onClick={() => {
              run(apply);
            }}
          >
            Apply
          </Button>
        )}
      </div>
      {dirty && blocked !== null && <Hint>{blocked}</Hint>}
      {message !== null && <InlineError>{message}</InlineError>}
    </div>
  );
}

/**
 * Servers: one row per city, not per server, with the region
 * picker's checkboxes when this build can change the firewall. Pings are
 * the game's own background checks (passive, nothing sent).
 */
export function ServersView() {
  const { settings, update } = useSettings();
  const snapshot = useLatestSnapshot();
  const [search, setSearch] = useState("");
  const [continent, setContinent] = useState<Continent | "all">("all");
  const [minBars, setMinBars] = useState<number | null>(null);
  const [maxMs, setMaxMs] = useState<number | null>(null);
  const [sorting, setSorting] = useState<SortingState>([{ id: "ping", desc: false }]);
  const view = settings.serversView;
  const picker = usePicker();
  // Keys the player has unticked but not applied yet; null = as applied.
  const [draft, setDraft] = useState<Set<string> | null>(null);

  // The game's pings change every few seconds; re-sorting live would make
  // cities jump around. Show a snapshot instead: taken when data first
  // arrives, then only when the player presses Refresh.
  const latest = snapshot?.gamePings?.regions;
  const [pinned, setPinned] = useState<typeof latest | null>(null);
  // Every city's last seen ping, so a switched-off one keeps its place.
  const [known, setKnown] = useState<Map<string, Region>>(new Map());
  const pin = (regions: NonNullable<typeof latest>) => {
    setPinned(regions);
    setKnown((prev) => {
      const next = new Map(prev);
      for (const r of mergeRegions(regions, [], new Map())) next.set(r.key, r);
      return next;
    });
  };
  // First data in: pin it straight away (adjusting state during render,
  // React's pattern for state derived from changing input).
  if (pinned == null && latest !== undefined && latest.length > 0) pin(latest);
  const live = useMemo(() => pinned ?? [], [pinned]);
  const appliedOff = useMemo(() => picker?.off ?? [], [picker]);
  const regions = useMemo(() => mergeRegions(live, appliedOff, known), [live, appliedOff, known]);
  const appliedKeys = new Set(appliedOff.map((a) => a.key));
  const offKeys = draft ?? appliedKeys;
  const dirty =
    draft !== null &&
    (draft.size !== appliedKeys.size || [...draft].some((k) => !appliedKeys.has(k)));
  const toggle: PickerSwitches | null = picker?.available
    ? {
        isOn: (r) => !offKeys.has(r.key),
        set: (r, on) => {
          const next = new Set(offKeys);
          if (on) next.delete(r.key);
          else next.add(r.key);
          setDraft(next);
        },
        allOn: regions.every((r) => !offKeys.has(r.key)),
        // Off: a starting point for "only these few": switch some back on,
        // then Apply (Apply stays blocked while every city is off).
        setAll: (on) => {
          setDraft(on ? new Set() : new Set(regions.map((r) => r.key)));
        },
      }
    : null;
  // Guard rails (the app checks them again): live capture, no match in
  // progress, and at least one city on (one the game is checking, if any).
  const pinged = regions.filter((r) => r.pingMs !== null);
  const matchNow = (snapshot?.measurement ?? null) !== null || picker?.matchActive === true;
  const blocked =
    offKeys.size === 0
      ? null
      : snapshot?.source !== "live"
        ? "Switching servers off needs live capture, so efsp can see when a match starts."
        : matchNow
          ? "Changes wait until this match ends."
          : !regions.some((r) => !offKeys.has(r.key)) ||
              (pinged.length > 0 && pinged.every((r) => offKeys.has(r.key)))
            ? "Leave at least one server the game is checking switched on."
            : picker?.gameExe === null
              ? "Find eFootball.exe first."
              : null;
  const apply = async () => {
    const off: Area[] = regions
      .filter((r) => offKeys.has(r.key))
      .map((r) => ({ key: r.key, name: r.name }));
    const done = await applyPicker(off);
    if (done) setDraft(null);
    return done;
  };

  const continents = [
    ...new Set(regions.flatMap((r) => (r.continent === null ? [] : [r.continent]))),
  ];
  // A choice whose regions have gone falls back to All, as the select shows.
  const continentNow = continent !== "all" && continents.includes(continent) ? continent : "all";
  const query = search.trim().toLowerCase();
  // Switched-off cities always stay in view, so they can be turned back on.
  const shown = regions.filter(
    (r) =>
      offKeys.has(r.key) ||
      ((query === "" || r.name.toLowerCase().includes(query)) &&
        (continentNow === "all" || r.continent === continentNow) &&
        (minBars === null || (r.bars !== null && r.bars >= minBars)) &&
        (maxMs === null || (r.pingMs !== null && r.pingMs <= maxMs))),
  );
  // TanStack Table returns a mutable object, which the React-compiler lint
  // rule flags; efsp doesn't use the compiler, so this only quiets the rule.
  // eslint-disable-next-line react-hooks/incompatible-library
  const table = useReactTable({
    data: sortRegions(shown, sorting[0] ?? { id: "ping", desc: false }),
    columns: COLUMNS,
    state: { sorting, columnVisibility: { on: toggle !== null } },
    meta: { serverToggle: toggle },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getRowId: (r) => r.key,
    // efsp sorts (sortRegions); the table tracks which column and way.
    manualSorting: true,
    enableSortingRemoval: false,
    enableMultiSort: false,
    // Low ping and A–Z first.
    sortDescFirst: false,
  });
  const sorted = table.getRowModel().rows.map((row) => row.original);

  const setView = (v: View) => {
    void update({ ...settings, serversView: v });
  };

  return (
    <Page>
      <PageHeader title="Servers">
        <Button
          variant="quiet"
          disabled={latest === undefined || latest.length === 0}
          onClick={() => {
            if (latest !== undefined) pin(latest);
          }}
        >
          <RefreshIcon />
          Refresh
        </Button>
        <ViewToggle label="Show servers as" value={view} onChange={setView} />
      </PageHeader>

      <FilterBar>
        <Input
          type="search"
          placeholder="Search servers"
          aria-label="Search servers"
          value={search}
          onChange={(e) => {
            setSearch(e.currentTarget.value);
          }}
        />
        <Select
          label="Continent"
          value={continentNow}
          options={[
            { value: "all", label: "All continents" },
            ...continents.map((c) => ({ value: c, label: CONTINENTS[c] })),
          ]}
          onValueChange={setContinent}
        />
        <Select
          label="Bars"
          value={minBars === null ? "any" : String(minBars)}
          options={MIN_BARS.map((b) => ({
            value: b.bars === null ? "any" : String(b.bars),
            label: b.label,
          }))}
          onValueChange={(v) => {
            setMinBars(v === "any" ? null : Number(v));
          }}
        />
        <Select
          label="Ping"
          value={maxMs === null ? "any" : String(maxMs)}
          options={MAX_PING.map((m) => ({
            value: m.ms === null ? "any" : String(m.ms),
            label: m.label,
          }))}
          onValueChange={(v) => {
            setMaxMs(v === "any" ? null : Number(v));
          }}
        />
      </FilterBar>

      {picker !== null && (
        <PickerPanel
          status={picker}
          dirty={dirty}
          blocked={blocked}
          apply={apply}
          reset={() => {
            setDraft(null);
          }}
        />
      )}

      {regions.length === 0 ? (
        <Empty>Servers appear while eFootball is running.</Empty>
      ) : shown.length === 0 ? (
        <Empty>No servers match these filters.</Empty>
      ) : view === "rows" ? (
        <DataTable table={table} caption="Servers" />
      ) : (
        <Cards regions={sorted} toggle={toggle} />
      )}
    </Page>
  );
}
