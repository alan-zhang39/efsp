import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { invoke, npcapStatusIs, resetTauriMock } from "./test/tauriMock";

vi.mock("@tauri-apps/api/core", async () => (await import("./test/tauriMock")).tauriMock);

describe("App: Npcap gate", () => {
  beforeEach(resetTauriMock);

  it("goes to the app once Npcap is found", async () => {
    npcapStatusIs({ state: "found", version: "1.88" });
    render(<App />);
    expect(await screen.findByRole("navigation", { name: "Main" })).toBeVisible();
    // Capture can start a moment after the nav first appears.
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("start_monitor", expect.anything());
    });
  });

  it("goes to the monitor in dev mode (not Windows)", async () => {
    npcapStatusIs({ state: "notApplicable" });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Starting capture" })).toBeVisible();
  });

  it("offers the official download when Npcap is missing", async () => {
    npcapStatusIs({ state: "missing", partial: false });
    render(<App />);
    await screen.findByRole("heading", { name: "Install Npcap to continue" });
    await userEvent.click(screen.getByRole("button", { name: "Get Npcap" }));
    expect(invoke).toHaveBeenCalledWith("open_npcap_download");
    expect(invoke).not.toHaveBeenCalledWith("start_monitor", expect.anything());
  });

  it("tells the user to reinstall on a partial install", async () => {
    npcapStatusIs({ state: "missing", partial: true });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Npcap needs reinstalling" })).toBeVisible();
  });

  it("asks for a reinstall when Npcap is limited to administrators", async () => {
    npcapStatusIs({ state: "restricted", version: "1.88" });
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Npcap is limited to administrators" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Check again" })).toBeVisible();
  });

  it("moves on to the monitor after the user installs Npcap and checks again", async () => {
    npcapStatusIs({ state: "missing", partial: false });
    render(<App />);
    await screen.findByRole("heading", { name: "Install Npcap to continue" });
    npcapStatusIs({ state: "found", version: "1.88" });
    await userEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByRole("navigation", { name: "Main" })).toBeVisible();
  });

  it("shows a clear failure when the check errors", async () => {
    invoke.mockRejectedValue(new Error("command npcap_status not allowed"));
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Couldn't check for Npcap" })).toBeVisible();
    expect(screen.getByText("command npcap_status not allowed")).toBeVisible();
  });

  it("reports when the browser can't be opened", async () => {
    invoke.mockImplementation((cmd) =>
      cmd === "npcap_status"
        ? Promise.resolve({ state: "missing", partial: false })
        : Promise.reject(new Error("no browser")),
    );
    render(<App />);
    await userEvent.click(await screen.findByRole("button", { name: "Get Npcap" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/npcap\.com/);
  });
});
