import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsProvider } from "../lib/SettingsContext";
import { DEFAULT_SETTINGS } from "../lib/settings";
import { resetLiveStore } from "../lib/liveStore";
import { resetPicker } from "../lib/picker";
import { invoke, lastChannel, resetTauriMock } from "../test/tauriMock";
import { choose } from "../test/ui";
import { Shell } from "./Shell";

vi.mock("@tauri-apps/api/core", async () => (await import("../test/tauriMock")).tauriMock);

const record = (id: number, kind: "match" | "offer", medianMs: number) => ({
  id,
  startedAt: Math.floor(Date.now() / 1000) - id * 3600,
  durationMs: kind === "offer" ? 9000 : 612_000,
  kind,
  method: "icmp",
  medianMs,
  p95Ms: medianMs + 6,
  jitterMs: 4,
  lossPct: 0,
  stalls: 0,
  bars: 4,
  ratePps: 52,
  spark: [medianMs, medianMs + 2, null, medianMs - 1],
  // Even ids on a Frankfurt server, odd ones peer-to-peer from France.
  endpoint:
    id % 2 === 0
      ? {
          kind: "server",
          region: "europe-west3",
          place: { city: "Frankfurt", country: "DE", continent: "europe" },
        }
      : { kind: "peer", country: "FR" },
  protocol: "udp",
});

let saved = structuredClone(DEFAULT_SETTINGS);

const frankfurt = {
  kind: "server",
  region: "europe-west3",
  place: { city: "Frankfurt", country: "DE", continent: "europe" },
};

function backend(records: unknown[]) {
  invoke.mockImplementation((cmd, args) => {
    switch (cmd) {
      case "settings_get":
        return Promise.resolve(saved);
      case "settings_set":
        saved = (args as { settings: typeof saved }).settings;
        return Promise.resolve(saved);
      case "history_list":
        return Promise.resolve({ records, total: records.length });
      default:
        return Promise.resolve(undefined);
    }
  });
}

function renderShell() {
  render(
    <SettingsProvider>
      <Shell npcap={{ state: "found", version: "1.88" }} />
    </SettingsProvider>,
  );
}

describe("Shell", () => {
  beforeEach(() => {
    resetTauriMock();
    resetLiveStore();
    resetPicker();
    saved = structuredClone(DEFAULT_SETTINGS);
  });

  it("navigates between Live, History and Settings", async () => {
    backend([]);
    renderShell();
    const nav = screen.getByRole("navigation", { name: "Main" });
    expect(within(nav).getByRole("button", { name: "Live" })).toHaveAttribute(
      "aria-current",
      "page",
    );

    await userEvent.click(within(nav).getByRole("button", { name: "History" }));
    expect(await screen.findByRole("heading", { name: "Match history" })).toBeVisible();
    expect(await screen.findByText(/No matches yet/)).toBeVisible();

    await userEvent.click(within(nav).getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
  });

  it("collapses the sidebar to icons, keeping accessible names", async () => {
    backend([]);
    renderShell();
    await userEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
    const nav = screen.getByRole("navigation", { name: "Main" });
    expect(within(nav).getByRole("button", { name: "History" })).toBeVisible();
    expect(within(nav).queryByText("History")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Expand sidebar" })).toBeVisible();
  });

  it("saves settings changes and applies the theme", async () => {
    backend([]);
    renderShell();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await choose("Theme", "Light");
    await vi.waitFor(() => {
      expect(saved.theme).toBe("light");
    });
    expect(document.documentElement.dataset.theme).toBe("light");

    const tray = screen.getByRole("switch", { name: "Start in the tray" });
    expect(tray).toBeDisabled();
    await userEvent.click(screen.getByRole("switch", { name: "Start with Windows" }));
    await vi.waitFor(() => {
      expect(saved.startWithWindows).toBe(true);
    });
    expect(tray).toBeEnabled();
  });

  it("lists history as rows or cards, and opens a match", async () => {
    backend([record(1, "match", 42), record(2, "offer", 61)]);
    renderShell();
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    const table = await screen.findByRole("table", { name: /Saved matches/ });
    expect(screen.getByText("1–2 of 2")).toBeVisible();
    expect(within(table).getAllByRole("row")).toHaveLength(3); // header + 2
    expect(within(table).getByText("Offer")).toBeVisible();

    await userEvent.click(screen.getByRole("button", { name: "Cards" }));
    await vi.waitFor(() => {
      expect(saved.historyView).toBe("cards");
    });
    expect(screen.queryByRole("table", { name: /Saved matches/ })).not.toBeInTheDocument();

    invoke.mockImplementation((cmd) =>
      Promise.resolve(
        cmd === "history_detail"
          ? { record: record(1, "match", 42), timeline: [] }
          : cmd === "settings_get"
            ? saved
            : { records: [record(1, "match", 42)], total: 1 },
      ),
    );
    const [firstCard] = screen.getAllByRole("button", { name: /42 ms/ });
    if (firstCard === undefined) throw new Error("no card for the 42 ms match");
    await userEvent.click(firstCard);
    expect(await screen.findByRole("button", { name: "All matches" })).toBeVisible();
    // An old match isn't compared with today's usual; it's shown as a fact.
    // Just the numbers: no "usual", no notes.
    expect(screen.queryByText(/usual/)).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("reloads history when a match or offer is saved", async () => {
    backend([record(1, "match", 42)]);
    renderShell();
    await vi.waitFor(() => {
      expect(() => lastChannel()).not.toThrow();
    });
    const snapshot = (saved: number) => ({
      source: "live",
      game: "running",
      gamePorts: 2,
      flows: [],
      recording: null,
      measurement: null,
      lastMatch: null,
      saved,
      gamePings: null,
      devTools: false,
      error: null,
    });
    act(() => {
      lastChannel().onmessage(snapshot(0));
    });
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    await screen.findByText("1–1 of 1");
    backend([record(1, "match", 42), record(2, "offer", 61)]);
    act(() => {
      lastChannel().onmessage(snapshot(1));
    });
    expect(await screen.findByText("1–2 of 2")).toBeVisible();
  });

  it("shows where each match was, and filters, sorts and pages in the app", async () => {
    backend([record(1, "match", 42), record(2, "match", 30), record(4, "match", 34)]);
    renderShell();
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    const table = await screen.findByRole("table", { name: /Saved matches/ });
    expect(within(table).getAllByText("Frankfurt")).toHaveLength(2);
    expect(within(table).getByText("France (peer)")).toBeVisible();
    const lastQuery = () =>
      invoke.mock.calls.filter(([cmd]) => cmd === "history_list").at(-1)?.[1] as {
        query: Record<string, unknown>;
      };

    // Filters, sorting and paging are asked of the app, not done on one page.
    await choose("Server or peer", "Peers only");
    await vi.waitFor(() => {
      expect(lastQuery().query).toMatchObject({ kind: "peer", offset: 0 });
    });
    await userEvent.type(screen.getByRole("searchbox", { name: "Search servers" }), "frank");
    await vi.waitFor(() => {
      expect(lastQuery().query).toMatchObject({ city: "frank" });
    });
    await userEvent.click(screen.getByRole("button", { name: "Ping" }));
    await vi.waitFor(() => {
      expect(lastQuery().query).toMatchObject({ sort: "ping", ascending: true });
    });
    expect(screen.getByRole("columnheader", { name: "Ping" })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );

    // The same column flips; a new one starts its natural way (longest).
    await userEvent.click(screen.getByRole("button", { name: "Ping" }));
    await vi.waitFor(() => {
      expect(lastQuery().query).toMatchObject({ sort: "ping", ascending: false });
    });
    await userEvent.click(screen.getByRole("button", { name: "Length" }));
    await vi.waitFor(() => {
      expect(lastQuery().query).toMatchObject({ sort: "length", ascending: false });
    });

    // One column at a time, even with Shift held.
    await userEvent.keyboard("{Shift>}");
    await userEvent.click(screen.getByRole("button", { name: "Date" }));
    await userEvent.keyboard("{/Shift}");
    await vi.waitFor(() => {
      expect(lastQuery().query).toMatchObject({ sort: "when", ascending: false });
    });
    const sorted = screen
      .getAllByRole("columnheader")
      .filter((h) => h.hasAttribute("aria-sort"))
      .map((h) => h.textContent);
    expect(sorted).toEqual(["Date↓"]);

    // Pressing the chosen view again keeps it.
    await userEvent.click(screen.getByRole("button", { name: "Rows" }));
    expect(screen.getByRole("button", { name: "Rows" })).toHaveAttribute("aria-pressed", "true");
  });

  it("lists server cities with the game's ping, searchable and sortable", async () => {
    backend([]);
    renderShell();
    // The Live view publishes snapshots the Servers page reads.
    await vi.waitFor(() => {
      expect(() => lastChannel()).not.toThrow();
    });
    act(() => {
      lastChannel().onmessage({
        source: "live",
        game: "running",
        gamePorts: 2,
        flows: [],
        recording: null,
        measurement: null,
        lastMatch: null,
        saved: 0,
        gamePings: {
          servers: 3,
          samples: 30,
          nearestMs: 19,
          typicalMs: 40,
          regions: [
            { endpoint: frankfurt, servers: 2, bestMs: 19.4, bars: 5 },
            {
              endpoint: {
                kind: "server",
                region: "eu-west-2",
                place: { city: "London", country: "GB", continent: "europe" },
              },
              servers: 1,
              bestMs: 31,
              bars: 4,
            },
            {
              endpoint: { kind: "unknown", network: null, country: null },
              servers: 4,
              bestMs: 50,
              bars: 3,
            },
          ],
        },
        devTools: false,
        error: null,
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Servers" }));
    const names = () =>
      within(screen.getByRole("table", { name: "Servers" }))
        .getAllByRole("row")
        .slice(1)
        .map((r) => within(r).getByRole("rowheader").textContent);
    await screen.findByRole("table", { name: "Servers" });
    // Cities only, never region codes, lowest ping first.
    expect(names()).toEqual(["Frankfurt", "London"]);

    // New pings don't reshuffle the list; Refresh takes them in.
    act(() => {
      lastChannel().onmessage({
        source: "live",
        game: "running",
        gamePorts: 2,
        flows: [],
        recording: null,
        measurement: null,
        lastMatch: null,
        saved: 0,
        gamePings: {
          servers: 2,
          samples: 30,
          nearestMs: 10,
          typicalMs: 15,
          regions: [
            {
              endpoint: {
                kind: "server",
                region: "eu-west-2",
                place: { city: "London", country: "GB", continent: "europe" },
              },
              servers: 1,
              bestMs: 10,
              bars: 4,
            },
            { endpoint: frankfurt, servers: 2, bestMs: 19.4, bars: 5 },
          ],
        },
        devTools: false,
        error: null,
      });
    });
    expect(names()).toEqual(["Frankfurt", "London"]);
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(names()).toEqual(["London", "Frankfurt"]);

    await userEvent.click(screen.getByRole("button", { name: "Ping" }));
    expect(names()).toEqual(["Frankfurt", "London"]);
    await userEvent.type(screen.getByRole("searchbox", { name: "Search servers" }), "lon");
    expect(names()).toEqual(["London"]);
    await userEvent.clear(screen.getByRole("searchbox", { name: "Search servers" }));

    // Bars filter: London has 4 bars, Frankfurt 5.
    await choose("Bars", "5 bars");
    const filtered = within(screen.getByRole("table")).getAllByRole("row").slice(1);
    expect(filtered.map((r) => within(r).getByRole("rowheader").textContent)).toEqual([
      "Frankfurt",
    ]);
    await choose("Ping", "Up to 50 ms");
    await choose("Continent", "Europe");
    expect(within(screen.getByRole("table")).getAllByRole("row")).toHaveLength(2);
  });

  it("keeps servers without a ping last, whichever way ping is sorted", async () => {
    invoke.mockImplementation((cmd) =>
      Promise.resolve(
        cmd === "settings_get"
          ? saved
          : cmd === "picker_status"
            ? {
                // Tokyo and Oslo are switched off, so the game doesn't ping them.
                off: [
                  { key: "server:Tokyo|JP", name: "Tokyo" },
                  { key: "server:Oslo|NO", name: "Oslo" },
                ],
                rulesOn: true,
                gameExe: "C:\\Steam\\eFootball.exe",
                available: true,
                matchActive: false,
              }
            : undefined,
      ),
    );
    renderShell();
    await vi.waitFor(() => {
      expect(() => lastChannel()).not.toThrow();
    });
    const london = {
      ...frankfurt,
      region: "europe-west2",
      place: { ...frankfurt.place, city: "London", country: "GB" },
    };
    act(() => {
      lastChannel().onmessage({
        source: "live",
        game: "running",
        gamePorts: 2,
        flows: [],
        recording: null,
        measurement: null,
        lastMatch: null,
        saved: 0,
        gamePings: {
          servers: 2,
          samples: 20,
          nearestMs: 19,
          typicalMs: 25,
          regions: [
            { endpoint: frankfurt, servers: 1, bestMs: 19, bars: 5 },
            { endpoint: london, servers: 1, bestMs: 31, bars: 4 },
          ],
        },
        devTools: false,
        error: null,
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Servers" }));
    const names = async () => {
      const table = await screen.findByRole("table", { name: "Servers" });
      return within(table)
        .getAllByRole("rowheader")
        .map((h) => h.textContent.replace(" (off)", ""));
    };
    await vi.waitFor(async () => {
      expect(await names()).toEqual(["Frankfurt", "London", "Oslo", "Tokyo"]);
    });
    await userEvent.click(screen.getByRole("button", { name: "Ping" }));
    expect(await names()).toEqual(["London", "Frankfurt", "Oslo", "Tokyo"]);
  });

  it("switches cities off with the picker and always offers the way back", async () => {
    const london = {
      kind: "server",
      region: "eu-west-2",
      place: { city: "London", country: "GB", continent: "europe" },
    };
    const status = (off: { key: string; name: string }[], matchActive = false) => ({
      off,
      rulesOn: off.length > 0,
      gameExe: "C:\\Steam\\eFootball.exe",
      available: true,
      matchActive,
    });
    let current = status([]);
    const calls: { cmd: string; args?: unknown }[] = [];
    invoke.mockImplementation((cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "settings_get":
          return Promise.resolve(saved);
        case "picker_status":
          return Promise.resolve(current);
        case "picker_apply":
          current = status((args as { off: { key: string; name: string }[] }).off);
          return Promise.resolve(current);
        case "picker_remove_all":
          current = status([]);
          return Promise.resolve(current);
        default:
          return Promise.resolve(undefined);
      }
    });
    renderShell();
    await vi.waitFor(() => {
      expect(() => lastChannel()).not.toThrow();
    });
    act(() => {
      lastChannel().onmessage({
        source: "live",
        game: "running",
        gamePorts: 2,
        flows: [],
        recording: null,
        measurement: null,
        lastMatch: null,
        saved: 0,
        gamePings: {
          servers: 2,
          samples: 20,
          nearestMs: 19,
          typicalMs: 25,
          regions: [
            { endpoint: frankfurt, servers: 1, bestMs: 19, bars: 5 },
            { endpoint: london, servers: 1, bestMs: 31, bars: 4 },
          ],
        },
        devTools: false,
        error: null,
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Servers" }));
    await screen.findByRole("table", { name: "Servers" });
    // Nothing to apply until something changes.
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();

    // Can't switch off every city the game is checking.
    await userEvent.click(screen.getByRole("switch", { name: "Allow matches in Frankfurt" }));
    await userEvent.click(screen.getByRole("switch", { name: "Allow matches in London" }));
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    expect(screen.getByText(/Leave at least one server/)).toBeVisible();
    await userEvent.click(screen.getByRole("switch", { name: "Allow matches in Frankfurt" }));

    // Reset drops unapplied changes.
    await userEvent.click(screen.getByRole("button", { name: "Reset" }));
    expect(screen.getByRole("switch", { name: "Allow matches in London" })).toBeChecked();
    expect(screen.queryByRole("button", { name: "Apply" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: "Allow matches in London" }));

    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(calls.find((c) => c.cmd === "picker_apply")?.args).toEqual({
      off: [{ key: "server:London|GB", name: "London" }],
    });
    expect(await screen.findByRole("button", { name: /1 server off/ })).toBeVisible();

    // The sidebar reminder shows on every page and opens Servers.
    await userEvent.click(screen.getByRole("button", { name: "History" }));
    await userEvent.click(screen.getByRole("button", { name: "1 server off" }));
    expect(await screen.findByRole("table", { name: "Servers" })).toBeVisible();

    // London isn't pinged any more (it's blocked); after Refresh it keeps the
    // ping it had and its place.
    act(() => {
      lastChannel().onmessage({
        source: "live",
        game: "running",
        gamePorts: 2,
        flows: [],
        recording: null,
        measurement: null,
        lastMatch: null,
        saved: 0,
        gamePings: {
          servers: 1,
          samples: 10,
          nearestMs: 19,
          typicalMs: 19,
          regions: [{ endpoint: frankfurt, servers: 1, bestMs: 19, bars: 5 }],
        },
        devTools: false,
        error: null,
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(screen.getByText("31 ms")).toBeVisible();

    // The all-switch turns every city back on; Apply removes the rules.
    await userEvent.click(screen.getByRole("switch", { name: "All servers" }));
    expect(screen.getByRole("switch", { name: "Allow matches in London" })).toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    await vi.waitFor(() => {
      expect(screen.queryByRole("button", { name: /server off/ })).not.toBeInTheDocument();
    });
    expect(calls.filter((c) => c.cmd === "picker_apply").at(-1)?.args).toEqual({ off: [] });
  });

  it("refuses to change rules during a match", async () => {
    invoke.mockImplementation((cmd) => {
      switch (cmd) {
        case "settings_get":
          return Promise.resolve(saved);
        case "picker_status":
          return Promise.resolve({
            off: [],
            rulesOn: false,
            gameExe: "C:\\G\\eFootball.exe",
            available: true,
            matchActive: true,
          });
        default:
          return Promise.resolve(undefined);
      }
    });
    renderShell();
    await vi.waitFor(() => {
      expect(() => lastChannel()).not.toThrow();
    });
    // Live capture, with a match being measured right now.
    act(() => {
      lastChannel().onmessage({
        source: "live",
        game: "running",
        gamePorts: 2,
        flows: [],
        recording: null,
        measurement: {
          flowId: 1,
          method: "icmp",
          ping: { lastMs: 20, medianMs: 20, p95Ms: 22, jitterMs: 1, lossPct: 0, probes: 10 },
          passive: { ratePps: 50, jitterMs: 1, stalls: 0, longestGapMs: 30 },
          probing: true,
          bars: 5,
          durationMs: 5000,
          recent: [],
          endpoint: frankfurt,
          protocol: "udp",
        },
        lastMatch: null,
        saved: 0,
        gamePings: {
          servers: 2,
          samples: 20,
          nearestMs: 19,
          typicalMs: 20,
          regions: [
            { endpoint: frankfurt, servers: 1, bestMs: 19, bars: 5 },
            {
              endpoint: {
                kind: "server",
                region: "eu-west-2",
                place: { city: "London", country: "GB", continent: "europe" },
              },
              servers: 1,
              bestMs: 31,
              bars: 4,
            },
          ],
        },
        devTools: false,
        error: null,
      });
    });
    await userEvent.click(screen.getByRole("button", { name: "Servers" }));
    await userEvent.click(
      await screen.findByRole("switch", { name: "Allow matches in Frankfurt" }),
    );
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
    expect(screen.getByText("Changes wait until this match ends.")).toBeVisible();
  });

  it("rolls a setting back if saving fails", async () => {
    invoke.mockImplementation((cmd) =>
      cmd === "settings_get"
        ? Promise.resolve(saved)
        : cmd === "settings_set"
          ? Promise.reject(new Error("Windows refused the startup entry"))
          : Promise.resolve(undefined),
    );
    renderShell();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const box = screen.getByRole("switch", { name: "Start with Windows" });
    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent("Windows refused the startup entry");
    expect(box).not.toBeChecked();
  });
});
