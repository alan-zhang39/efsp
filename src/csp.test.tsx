import { CSPProvider } from "@base-ui/react/csp-provider";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { Shell } from "./components/Shell";
import { TooltipProvider } from "./components/ui/tooltip";
import { SettingsProvider } from "./lib/SettingsContext";
import { resetTauriMock } from "./test/tauriMock";
import { choose } from "./test/ui";

vi.mock("@tauri-apps/api/core", async () => (await import("./test/tauriMock")).tauriMock);

beforeEach(resetTauriMock);

// The release build's CSP forbids inline <style>. With the provider set up
// as in main.tsx, opening popups must not add any.
it("adds no inline styles with the production CSP setup", async () => {
  render(
    <CSPProvider disableStyleElements>
      <TooltipProvider>
        <SettingsProvider>
          <Shell npcap={{ state: "found", version: "1.88" }} />
        </SettingsProvider>
      </TooltipProvider>
    </CSPProvider>,
  );
  await userEvent.click(screen.getByRole("button", { name: "Settings" }));
  await choose("Theme", "Light");
  await userEvent.click(screen.getByRole("button", { name: "Collapse sidebar" }));
  await userEvent.hover(screen.getByRole("button", { name: "History" }));
  expect(document.querySelectorAll("style")).toHaveLength(0);
});
