import {
  createColumnHelper,
  getCoreRowModel,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import { cn } from "cn";
import { useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { Toggle } from "@/components/ui/toggle";
import { formatDuration, tierFor } from "../lib/bars";
import { placeName } from "../lib/endpoint";
import { formatDate, formatMs, formatTime } from "../lib/format";
import { useLatestSnapshot } from "../lib/liveStore";
import {
  exportCsv,
  historyDetail,
  type HistoryPage,
  type HistorySort,
  listHistory,
  type MatchRecord,
} from "../lib/history";
import type { Measurement, Point } from "../lib/monitor";
import type { HistoryView as View } from "../lib/settings";
import { useSettings } from "../lib/SettingsContext";
import { BarsGlyph, PingBars, TIER_TEXT } from "./BarsGlyph";
import { DataTable, Pager } from "./DataTable";
import { MatchCard } from "./MatchCard";
import { Empty, FilterBar, Hint, InlineError, Page, PageHeader } from "./Page";
import { Sparkline } from "./Sparkline";
import { ViewToggle } from "./ViewToggle";

const PAGE = 20;

const RANGES = [
  { value: "7", label: "Last 7 days" },
  { value: "30", label: "Last 30 days" },
  { value: "90", label: "Last 90 days" },
  { value: "all", label: "All time" },
] as const;

const KINDS = [
  { value: "all", label: "Servers and peers" },
  { value: "server", label: "Servers only" },
  { value: "peer", label: "Peers only" },
] as const;

/**
 * A match's length. Offers (declined or timed out) always last a few
 * seconds, so their length says nothing: a dash. It's still stored and
 * exported.
 */
function lengthText(r: MatchRecord): string {
  return r.kind === "offer" ? "—" : formatDuration(r.durationMs);
}

const SORTS: Record<string, HistorySort> = { when: "when", ping: "ping", length: "length" };

/** Where a record was; "—" for records without a location. */
function serverText(r: MatchRecord): string {
  return r.endpoint === null ? "—" : placeName(r.endpoint);
}

function Tags({ r }: { r: MatchRecord }) {
  return (
    <>
      {r.kind === "offer" && <Badge>Offer</Badge>}
      {r.protocol === "tcp" && <Badge>TCP</Badge>}
    </>
  );
}

function Ping({ r, className }: { r: MatchRecord; className?: string | undefined }) {
  return r.medianMs === null ? (
    <span className="text-muted-foreground">—</span>
  ) : (
    <PingBars ms={r.medianMs} bars={r.bars} className={className} />
  );
}

const col = createColumnHelper<MatchRecord>();

/**
 * History's columns, defined once (see the Servers table for why). Sorting
 * runs in the app's database; the table only tracks which column and way.
 * Each sortable column starts in its natural direction: newest, lowest
 * ping, longest.
 */
const COLUMNS = [
  col.accessor("startedAt", {
    id: "when",
    header: "Date",
    sortDescFirst: true,
    // A real button, so the row opens from the keyboard too.
    cell: (c) => (
      <button
        type="button"
        className="cursor-pointer font-medium focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        onClick={(e) => {
          e.stopPropagation();
          c.table.options.meta?.openMatch?.(c.row.original.id);
        }}
      >
        {formatDate(c.getValue())}
      </button>
    ),
    meta: { rowHeader: true },
  }),
  col.accessor("startedAt", {
    id: "time",
    header: "Time",
    enableSorting: false,
    cell: (c) => formatTime(c.getValue()),
    meta: { className: "text-muted-foreground tabular-nums" },
  }),
  col.display({
    id: "server",
    header: "Server",
    cell: (c) => (
      <>
        {serverText(c.row.original)}
        <Tags r={c.row.original} />
      </>
    ),
  }),
  col.accessor("medianMs", {
    id: "ping",
    header: "Ping",
    sortDescFirst: false,
    cell: (c) => <Ping r={c.row.original} />,
  }),
  col.accessor("durationMs", {
    id: "length",
    header: "Length",
    sortDescFirst: true,
    cell: (c) => lengthText(c.row.original),
    meta: {
      numeric: true,
      // Hidden in a narrow window, as before.
      className: "text-muted-foreground tabular-nums max-[640px]:hidden",
    },
  }),
];

function Cards({ records, open }: { records: MatchRecord[]; open: (id: number) => void }) {
  return (
    <ul className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3 p-0">
      {records.map((r) => (
        <li key={r.id}>
          <button
            type="button"
            className="grid w-full cursor-pointer gap-2 rounded-[0.625rem] border border-line bg-background p-4 text-left hover:bg-surface focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            onClick={() => {
              open(r.id);
            }}
          >
            <span className="text-[0.8125rem] text-muted-foreground">
              {formatDate(r.startedAt)} · {formatTime(r.startedAt)}
              <Tags r={r} />
            </span>
            <span className="flex items-end justify-between">
              <span
                className={cn(
                  "text-[1.75rem] leading-none font-[650] tracking-[-0.02em] tabular-nums",
                  r.medianMs === null
                    ? "text-muted-foreground"
                    : r.bars !== null && TIER_TEXT[tierFor(r.bars)],
                )}
              >
                {r.medianMs === null ? "—" : formatMs(r.medianMs)}
              </span>
              {r.bars !== null && <BarsGlyph bars={r.bars} className="h-[1.4rem] w-7" />}
            </span>
            <Sparkline values={r.spark} />
            <span className="text-[0.8125rem] text-muted-foreground">
              {serverText(r)}
              {r.kind === "match" && ` · ${formatDuration(r.durationMs)}`}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/** A saved record as the match card expects it. */
function asMeasurement(r: MatchRecord): Measurement {
  return {
    flowId: r.id,
    method: r.method,
    ping:
      r.medianMs === null
        ? null
        : {
            lastMs: null,
            medianMs: r.medianMs,
            p95Ms: r.p95Ms,
            jitterMs: r.jitterMs ?? 0,
            lossPct: r.lossPct ?? 0,
            probes: 0,
          },
    passive: { ratePps: r.ratePps, jitterMs: 0, stalls: r.stalls, longestGapMs: 0 },
    probing: r.medianMs !== null,
    bars: r.bars,
    durationMs: r.durationMs,
    endpoint: r.endpoint,
    protocol: r.protocol,
    recent: [],
  };
}

/** One saved match (layout B): where, the ping and bars, when, the graph. */
function Detail({ id, back }: { id: number; back: () => void }) {
  const [data, setData] = useState<{ record: MatchRecord; timeline: Point[] } | null>(null);
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    let cancelled = false;
    historyDetail(id).then(
      (d) => {
        if (cancelled) return;
        if (d === null) setMissing(true);
        else setData(d);
      },
      () => {
        if (!cancelled) setMissing(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [id]);

  return (
    <Page>
      <Button variant="text" className="justify-self-start" onClick={back}>
        <span aria-hidden="true">←</span> All matches
      </Button>
      {missing && <Empty>This match isn't in your history any more.</Empty>}
      {data !== null && (
        <MatchCard
          m={asMeasurement(data.record)}
          mode="saved"
          timeline={data.timeline}
          meta={
            <div>
              <p className="m-0 text-base font-semibold">
                {formatDate(data.record.startedAt)}
                <Tags r={data.record} />
              </p>
              <p className="m-0 text-sm text-muted-foreground">
                {formatTime(data.record.startedAt)}
                {data.record.kind === "match" && ` · ${formatDuration(data.record.durationMs)}`}
              </p>
            </div>
          }
        />
      )}
    </Page>
  );
}

export function HistoryView() {
  const { settings, update } = useSettings();
  const [rangeDays, setRangeDays] = useState<number | null>(30);
  const [includeOffers, setIncludeOffers] = useState(true);
  const [kind, setKind] = useState<"server" | "peer" | null>(null);
  const [cityInput, setCityInput] = useState("");
  const [city, setCity] = useState("");
  const [sorting, setSorting] = useState<SortingState>([{ id: "when", desc: true }]);
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<HistoryPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<number | null>(null);
  const [exportNote, setExportNote] = useState<string | null>(null);
  const view = settings.historyView;
  // Bumped whenever a match or offer is saved, so the list stays current.
  const saved = useLatestSnapshot()?.saved ?? 0;

  // Search as you type, without a query per keystroke.
  useEffect(() => {
    const t = setTimeout(() => {
      setCity(cityInput.trim());
      setOffset(0);
    }, 250);
    return () => {
      clearTimeout(t);
    };
  }, [cityInput]);

  useEffect(() => {
    let cancelled = false;
    listHistory({
      sinceDays: rangeDays,
      includeOffers,
      limit: PAGE,
      offset,
      kind,
      city: city === "" ? null : city,
      sort: SORTS[sorting[0]?.id ?? "when"] ?? "when",
      ascending: sorting[0]?.desc === false,
    }).then(
      (p) => {
        if (!cancelled) {
          setPage(p);
          setError(null);
        }
      },
      (e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [rangeDays, includeOffers, kind, city, sorting, offset, openId, saved]);

  // TanStack Table returns a mutable object, which the React-compiler lint
  // rule flags; efsp doesn't use the compiler, so this only quiets the rule.
  // eslint-disable-next-line react-hooks/incompatible-library
  const table = useReactTable({
    data: page?.records ?? [],
    columns: COLUMNS,
    state: { sorting },
    onSortingChange: (next) => {
      setSorting(next);
      setOffset(0);
    },
    manualSorting: true,
    enableSortingRemoval: false,
    // One column at a time: the app's query sorts by one.
    enableMultiSort: false,
    getCoreRowModel: getCoreRowModel(),
    getRowId: (r) => String(r.id),
    meta: { openMatch: setOpenId },
  });

  if (openId !== null) {
    return (
      <Detail
        id={openId}
        back={() => {
          setOpenId(null);
        }}
      />
    );
  }

  const records = page?.records ?? [];
  const filtered = kind !== null || city !== "";
  const setView = (v: View) => {
    void update({ ...settings, historyView: v });
  };
  const doExport = () => {
    setExportNote(null);
    exportCsv().then(
      (saved) => {
        if (saved) setExportNote("Saved as CSV.");
      },
      (e: unknown) => {
        setExportNote(e instanceof Error ? e.message : String(e));
      },
    );
  };

  return (
    <Page>
      <PageHeader title="Match history">
        <Button variant="quiet" onClick={doExport}>
          Export CSV
        </Button>
        <ViewToggle label="Show history as" value={view} onChange={setView} />
      </PageHeader>

      <FilterBar>
        <Input
          type="search"
          placeholder="Search servers"
          aria-label="Search servers"
          value={cityInput}
          onChange={(e) => {
            setCityInput(e.currentTarget.value);
          }}
        />
        <Select
          label="Time range"
          value={rangeDays === null ? "all" : String(rangeDays)}
          options={RANGES}
          onValueChange={(v) => {
            setRangeDays(v === "all" ? null : Number(v));
            setOffset(0);
          }}
        />
        <Select
          label="Server or peer"
          value={kind ?? "all"}
          options={KINDS}
          onValueChange={(v) => {
            setKind(v === "all" ? null : v);
            setOffset(0);
          }}
        />
        <Toggle
          pressed={includeOffers}
          onPressedChange={(on) => {
            setIncludeOffers(on);
            setOffset(0);
          }}
        >
          <span className="inline-block w-[0.75em]" aria-hidden="true">
            {includeOffers ? "✓" : ""}
          </span>
          Offers
        </Toggle>
      </FilterBar>
      {exportNote !== null && <Hint>{exportNote}</Hint>}
      {error !== null && <InlineError>{error}</InlineError>}

      {page !== null && records.length === 0 && (
        <Empty>{filtered ? "No matches here." : "No matches yet."}</Empty>
      )}

      {records.length > 0 &&
        (view === "rows" ? (
          <DataTable
            table={table}
            caption="Saved matches"
            rowProps={(row) => ({
              className: "cursor-pointer hover:bg-surface",
              onClick: () => {
                setOpenId(row.original.id);
              },
            })}
          />
        ) : (
          <Cards records={records} open={setOpenId} />
        ))}

      {page !== null && (
        <Pager offset={offset} pageSize={PAGE} total={page.total} onOffset={setOffset} />
      )}
    </Page>
  );
}
