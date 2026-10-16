import { vi } from "vitest";

/**
 * Shared mock of `@tauri-apps/api/core`. Tests register it with
 * `vi.mock("@tauri-apps/api/core", () => tauriMock)` and push monitor
 * snapshots through `lastChannel()`.
 */
type Listener = (message: unknown) => void;

const channels: { onmessage: Listener }[] = [];

class MockChannel {
  onmessage: Listener = () => undefined;
  constructor() {
    channels.push(this);
  }
}

export const invoke = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
export const tauriMock = { invoke, Channel: MockChannel };

export function lastChannel(): { onmessage: Listener } {
  const ch = channels.at(-1);
  if (ch === undefined) throw new Error("no channel created yet");
  return ch;
}

export function resetTauriMock(): void {
  invoke.mockReset();
  channels.length = 0;
}

/** Answers `npcap_status` with `status`; every other command succeeds. */
export function npcapStatusIs(status: unknown): void {
  invoke.mockImplementation((cmd) => Promise.resolve(cmd === "npcap_status" ? status : undefined));
}
