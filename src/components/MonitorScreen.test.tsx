import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FlowView, Measurement, RegionPing, Snapshot } from "../lib/monitor";
import { invoke, lastChannel, npcapStatusIs, resetTauriMock } from "../test/tauriMock";
import { SettingsProvider } from "../lib/SettingsContext";
import { MonitorScreen } from "./MonitorScreen";

vi.mock("@tauri-apps/api/core", async () => (await import("../test/tauriMock")).tauriMock);

function flow(overrides: Partial<FlowView>): FlowView {
  return {
    id: 1,
    remoteLabel: 1,
    protocol: "udp",
    localPort: 50000,
    remotePort: 6000,
    ageMs: 20_000,
    idleMs: 0,
    packetsIn: 1200,
    packetsOut: 1180,
    bytesIn: 0,
    bytesOut: 0,
    ratePps: 60,
    sustained: false,
    idle: false,
    endpoint: { kind: "unknown", network: null, country: null },
    ...overrides,
  };
}

function snap(overrides: Partial<Snapshot>): Snapshot {
  return {
    source: "live",
    game: "running",
    gamePorts: 3,
    flows: [],
    recording: null,
    measurement: null,
    lastMatch: null,
    gamePings: null,
    saved: 0,
    tcpMatches: 0,
    devTools: true,
    error: null,
    ...overrides,
  };
}

async function renderWith(s: Snapshot) {
  render(
    <SettingsProvider>
      <MonitorScreen npcap={{ state: "found", version: "1.88" }} />
    </SettingsProvider>,
  );
  await vi.waitFor(() => {
    expect(invoke).toHaveBeenCalledWith("start_monitor", expect.anything());
  });
  act(() => {
    lastChannel().onmessage(s);
  });
}

function city(name: string, bestMs: number, bars: number): RegionPing {
  return {
    endpoint: {
      kind: "server",
      region: name.toLowerCase(),
      place: { city: name, country: "CA", continent: "northAmerica" },
    },
    servers: 1,
    bestMs,
    bars,
  };
}

function pings(regions: RegionPing[]): Snapshot["gamePings"] {
  return { servers: regions.length, samples: 10, nearestMs: null, typicalMs: null, regions };
}

describe("MonitorScreen", () => {
  beforeEach(() => {
    resetTauriMock();
    npcapStatusIs({ state: "found", version: "1.88" });
  });

  it("waits for the game", async () => {
    await renderWith(snap({ game: "notRunning", gamePorts: 0 }));
    expect(screen.getByRole("heading", { name: "Waiting for eFootball" })).toBeVisible();
    expect(screen.getByText("No connections yet.")).toBeVisible();
  });

  it("waits for a match while the game runs, with no connection list in release", async () => {
    await renderWith(snap({ devTools: false, flows: [flow({ id: 3, remoteLabel: 3 })] }));
    expect(screen.getByRole("heading", { name: "Waiting for a match" })).toBeVisible();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/Remote \d/);
  });

  it("lists the nearest servers while waiting, moving rows only for clear changes", async () => {
    const open = vi.fn();
    render(
      <SettingsProvider>
        <MonitorScreen npcap={{ state: "found", version: "1.88" }} onOpenServers={open} />
      </SettingsProvider>,
    );
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("start_monitor", expect.anything());
    });
    const regions = [
      city("Toronto", 26, 5),
      city("Montreal", 31, 5),
      city("Virginia", 48, 4),
      city("Iowa", 62, 3),
      city("Oregon", 88, 2),
      city("Tokyo", 160, 1),
    ];
    act(() => {
      lastChannel().onmessage(snap({ devTools: false, gamePings: pings(regions) }));
    });
    const list = screen.getByRole("region", { name: "Nearest servers" });
    const names = () =>
      within(list)
        .getAllByRole("listitem")
        .map((li) => li.firstElementChild?.textContent);
    expect(names()).toEqual(["Toronto", "Montreal", "Virginia", "Iowa", "Oregon"]);
    expect(within(list).getByText("26 ms")).toBeVisible();

    // Small changes: the numbers update, the rows stay put.
    const others = regions.filter((r) => r.bestMs !== 31);
    act(() => {
      lastChannel().onmessage(
        snap({ devTools: false, gamePings: pings([city("Montreal", 24, 5), ...others]) }),
      );
    });
    expect(names()).toEqual(["Toronto", "Montreal", "Virginia", "Iowa", "Oregon"]);
    expect(within(list).getByText("24 ms")).toBeVisible();

    // A clearly nearer city, or one that wasn't shown, re-sorts the list.
    act(() => {
      lastChannel().onmessage(
        snap({
          devTools: false,
          gamePings: pings([city("Montreal", 15, 5), city("London", 20, 5), ...others]),
        }),
      );
    });
    expect(names()).toEqual(["Montreal", "London", "Toronto", "Virginia", "Iowa"]);

    await userEvent.click(within(list).getByRole("button", { name: "All servers" }));
    expect(open).toHaveBeenCalled();
  });

  it("shows no server list while the game is closed", async () => {
    await renderWith(
      snap({ devTools: false, game: "notRunning", gamePings: pings([city("Toronto", 26, 5)]) }),
    );
    expect(screen.queryByRole("region", { name: "Nearest servers" })).not.toBeInTheDocument();
  });

  it("shows the connection list only in dev builds", async () => {
    await renderWith(snap({ flows: [flow({ id: 3, remoteLabel: 3 })] }));
    const dev = screen.getByRole("region", { name: "Developer view" });
    expect(within(dev).getByRole("table")).toBeVisible();
  });

  it("tags new and idle connections in text, not colour alone", async () => {
    await renderWith(
      snap({
        flows: [
          flow({ id: 2, remoteLabel: 2, ageMs: 1000 }),
          flow({ id: 1, remoteLabel: 1, idle: true, ratePps: 0 }),
        ],
      }),
    );
    expect(screen.getByText("New")).toBeVisible();
    expect(screen.getByText("Idle")).toBeVisible();
  });

  it("never renders anything that looks like an IP address", async () => {
    await renderWith(snap({ flows: [flow({ sustained: true })] }));
    expect(document.body.textContent).not.toMatch(/\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}/);
  });

  it("explains when capture is unavailable", async () => {
    await renderWith(snap({ source: "unavailable", error: "Live capture runs on Windows." }));
    expect(screen.getByRole("heading", { name: "Live capture isn't running" })).toBeVisible();
    expect(screen.getByText("Live capture runs on Windows.")).toBeVisible();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("starts and stops recording", async () => {
    await renderWith(snap({}));
    await userEvent.click(screen.getByRole("button", { name: "Record trace" }));
    expect(invoke).toHaveBeenCalledWith("set_recording", { on: true });

    act(() => {
      lastChannel().onmessage(
        snap({ recording: { fileName: "efsp-trace-1.jsonl", packets: 1500 } }),
      );
    });
    expect(screen.getByText("Recording efsp-trace-1.jsonl, 1.5K packets")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Stop recording" }));
    expect(invoke).toHaveBeenCalledWith("set_recording", { on: false });
  });

  it("has no recorder in release builds", async () => {
    await renderWith(snap({ devTools: false }));
    expect(screen.queryByRole("button", { name: "Record trace" })).not.toBeInTheDocument();
    expect(screen.queryByText(/Open traces folder/)).not.toBeInTheDocument();
  });

  it("hides the recorder during replay", async () => {
    await renderWith(snap({ source: "replay" }));
    expect(screen.getByText("Replaying a trace")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Record trace" })).not.toBeInTheDocument();
  });

  const passive = { ratePps: 52.4, jitterMs: 4.2, stalls: 0, longestGapMs: 116 };
  function measured(m: Partial<Measurement>): Snapshot {
    return snap({
      flows: [flow({ id: 9, sustained: true })],
      measurement: {
        flowId: 9,
        method: "icmp",
        ping: null,
        passive,
        probing: true,
        bars: null,
        endpoint: null,
        protocol: "udp",
        durationMs: 8000,
        recent: [],
        ...m,
      },
    });
  }

  it("shows the match card: where, the ping and bars, jitter, loss and the graph", async () => {
    await renderWith(
      measured({
        bars: 5,
        ping: { lastMs: 33, medianMs: 24.4, p95Ms: 31, jitterMs: 4.8, lossPct: 0, probes: 50 },
        recent: [{ tMs: 250, rttMs: 24, jitterMs: 4.8, lost: 0, stalls: 0, ratePps: 54 }],
      }),
    );
    const card = screen.getByRole("region", { name: "Match connection" });
    expect(within(card).getByText("24 ms")).toBeVisible();
    expect(within(card).getByRole("img", { name: "5 of 5 bars" })).toBeVisible();
    // Jitter and loss as labelled values; packets/s hidden by default.
    expect(within(card).getByText("Jitter")).toBeVisible();
    expect(within(card).getByText("4.8 ms")).toBeVisible();
    expect(within(card).getByText("0%")).toBeVisible();
    expect(within(card).queryByText(/Packets/)).not.toBeInTheDocument();
    expect(within(card).getByText(/Live · 0:08/)).toBeVisible();
    // No explanations, no verdict words.
    expect(card.textContent).not.toMatch(/ICMP|median|usual/);
    expect(document.body.textContent).not.toMatch(/\b(Good|OK|Bad)\b/);
    expect(screen.getByTestId("graph")).toHaveAttribute("data-live", "true");
    expect(screen.getByTestId("graph")).toHaveAttribute("data-points", "1");
  });

  it("says where the match is: the server's city or the peer's country", async () => {
    const ping = { lastMs: 33, medianMs: 24.4, p95Ms: 31, jitterMs: 1, lossPct: 0, probes: 50 };
    const toronto = {
      kind: "server",
      region: "northamerica-northeast2",
      place: { city: "Toronto", country: "CA", continent: "northAmerica" },
    } as const;
    await renderWith(measured({ bars: 5, ping, endpoint: toronto }));
    expect(screen.getByRole("heading", { level: 1, name: "Toronto" })).toBeVisible();
    act(() => {
      lastChannel().onmessage(
        measured({ bars: 5, ping, endpoint: { kind: "peer", country: "CA" } }),
      );
    });
    expect(screen.getByRole("heading", { level: 1, name: "Canada (peer)" })).toBeVisible();
  });

  it("labels each connection's location in the dev list, and the game's pings by region", async () => {
    await renderWith(
      snap({
        flows: [flow({ id: 4, remoteLabel: 4, endpoint: { kind: "peer", country: "FR" } })],
        gamePings: {
          servers: 3,
          samples: 30,
          nearestMs: 19.6,
          typicalMs: 30,
          regions: [
            {
              endpoint: {
                kind: "server",
                region: "eu-central-1",
                place: { city: "Frankfurt", country: "DE", continent: "europe" },
              },
              servers: 2,
              bestMs: 19.6,
              bars: 5,
            },
          ],
        },
      }),
    );
    expect(screen.getByRole("cell", { name: "France (peer)" })).toBeVisible();
    expect(screen.getByText("Frankfurt: 20 ms (2 servers)")).toBeVisible();
  });

  it("labels a TCP match", async () => {
    const ping = { lastMs: 33, medianMs: 24.4, p95Ms: 31, jitterMs: 1, lossPct: 0, probes: 50 };
    await renderWith(measured({ bars: 5, ping, protocol: "tcp" }));
    const card = screen.getByRole("region", { name: "Match connection" });
    expect(within(card).getByText("TCP")).toBeVisible();
  });

  it("announces a new match once, politely, with its ping and bars", async () => {
    await renderWith(
      measured({
        bars: 5,
        ping: { lastMs: 24, medianMs: 24.4, p95Ms: 31, jitterMs: 4.8, lossPct: 0, probes: 50 },
      }),
    );
    const region = document.querySelector('[aria-live="polite"].sr-only');
    expect(region).toHaveTextContent("Match found: 24 ms, 5 of 5 bars.");
  });

  it("goes back to waiting once a match ends, leaving the last match to History", async () => {
    const finished = measured({
      bars: 4,
      durationMs: 725_000,
      ping: { lastMs: 45, medianMs: 44, p95Ms: 50, jitterMs: 3, lossPct: 0, probes: 50 },
    }).measurement;
    await renderWith(snap({ devTools: false, lastMatch: finished }));
    expect(screen.getByRole("heading", { name: "Waiting for a match" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Match" })).not.toBeInTheDocument();
    expect(screen.queryByText("44 ms")).not.toBeInTheDocument();
  });

  it("colours by bars but always shows the number and bar count", async () => {
    await renderWith(
      measured({
        bars: 2,
        ping: { lastMs: 110, medianMs: 112, p95Ms: 130, jitterMs: 6, lossPct: 0, probes: 50 },
      }),
    );
    const card = screen.getByRole("region", { name: "Match connection" });
    expect(card).toHaveAttribute("data-tier", "low");
    expect(within(card).getByText("112 ms")).toBeVisible();
    expect(within(card).getByRole("img", { name: "2 of 5 bars" })).toBeVisible();
  });

  it("marks a route estimate as approximate", async () => {
    await renderWith(
      measured({
        method: "routeEstimate",
        bars: 5,
        ping: { lastMs: 28, medianMs: 28, p95Ms: 30, jitterMs: 1, lossPct: 0, probes: 20 },
      }),
    );
    expect(screen.getByText("≈")).toBeVisible();
  });

  it("shows a dash, and no jitter or loss, without a ping number", async () => {
    await renderWith(measured({ method: "passive" }));
    const card = screen.getByRole("region", { name: "Match connection" });
    expect(within(card).getByText("—")).toBeVisible();
    expect(within(card).queryByText("Jitter")).not.toBeInTheDocument();
  });

  it("shows loss while ICMP has no replies yet, instead of hiding it", async () => {
    await renderWith(
      measured({
        ping: { lastMs: null, medianMs: null, p95Ms: null, jitterMs: 0, lossPct: 100, probes: 6 },
      }),
    );
    const card = screen.getByRole("region", { name: "Match connection" });
    expect(within(card).getByText("…")).toBeVisible();
    expect(within(card).getByText("100%")).toBeVisible();
  });

  it("compares with Windows ping while measuring with ICMP", async () => {
    await renderWith(
      measured({
        ping: { lastMs: 33, medianMs: 34.4, p95Ms: 41, jitterMs: 2.3, lossPct: 0, probes: 50 },
      }),
    );
    invoke.mockImplementation((cmd) =>
      Promise.resolve(
        cmd === "compare_with_ping"
          ? {
              efspMedianMs: 34.4,
              pingExeMedianMs: 33,
              pingExeReplies: 10,
              differenceMs: 1.4,
              withinTolerance: true,
            }
          : undefined,
      ),
    );
    await userEvent.click(screen.getByRole("button", { name: "Compare with Windows ping" }));
    expect(
      await screen.findByText(
        "efsp 34 ms, ping.exe 33 ms (10 replies): 1.4 ms apart, within ±5 ms.",
      ),
    ).toBeVisible();
  });

  it("only offers the ping comparison while measuring with ICMP", async () => {
    await renderWith(measured({ method: "routeEstimate" }));
    expect(screen.getByRole("button", { name: "Compare with Windows ping" })).toBeDisabled();
  });

  it("runs the game-ping experiment between matches", async () => {
    await renderWith(
      snap({
        gamePings: { servers: 61, samples: 900, nearestMs: 19.6, typicalMs: 151.6, regions: [] },
      }),
    );
    expect(
      screen.getByText("Game's own server pings: 61 servers, nearest 20 ms, typical 152 ms."),
    ).toBeVisible();
    invoke.mockImplementation((cmd) =>
      Promise.resolve(
        cmd === "compare_game_pings"
          ? [{ server: 1, gameMs: 19.6, icmpMs: 18.9, icmpReplies: 5, differenceMs: 0.7 }]
          : undefined,
      ),
    );
    await userEvent.click(screen.getByRole("button", { name: "Compare game pings with ICMP" }));
    expect(
      await screen.findByText("Server 1: game 20 ms, ICMP 19 ms (5 replies), 0.7 ms apart."),
    ).toBeVisible();
  });

  it("tests route probing between matches", async () => {
    await renderWith(snap({}));
    invoke.mockImplementation((cmd) =>
      Promise.resolve(cmd === "test_route_probe" ? "Works: the first router answered." : undefined),
    );
    await userEvent.click(screen.getByRole("button", { name: "Test route probing" }));
    expect(await screen.findByText("Works: the first router answered.")).toBeVisible();
  });

  it("surfaces a malformed snapshot as an error instead of rendering it", async () => {
    await renderWith({ nonsense: true } as unknown as Snapshot);
    expect(screen.getByRole("heading", { name: "Live capture isn't running" })).toBeVisible();
  });

  it("recovers when a good snapshot follows a malformed one", async () => {
    await renderWith({ nonsense: true } as unknown as Snapshot);
    expect(screen.getByRole("heading", { name: "Live capture isn't running" })).toBeVisible();
    const ping = { lastMs: 33, medianMs: 24.4, p95Ms: 31, jitterMs: 1, lossPct: 0, probes: 50 };
    act(() => {
      lastChannel().onmessage(measured({ bars: 5, ping }));
    });
    expect(screen.getByRole("region", { name: "Match connection" })).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Live capture isn't running" })).toBeNull();
  });

  it("drops a frozen match card when snapshots stop reading", async () => {
    const ping = { lastMs: 33, medianMs: 24.4, p95Ms: 31, jitterMs: 1, lossPct: 0, probes: 50 };
    await renderWith(measured({ bars: 5, ping }));
    expect(screen.getByRole("region", { name: "Match connection" })).toBeVisible();
    act(() => {
      lastChannel().onmessage({ nonsense: true });
    });
    expect(screen.queryByRole("region", { name: "Match connection" })).toBeNull();
    expect(screen.getByRole("heading", { name: "Live capture isn't running" })).toBeVisible();
  });
});
