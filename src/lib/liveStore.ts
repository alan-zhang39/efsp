import { useSyncExternalStore } from "react";
import type { Snapshot } from "./monitor";

/**
 * The latest monitor snapshot, shared between views. The Live view (always
 * mounted) owns the subscription and publishes here; other pages, like
 * Servers, read it without opening a second channel.
 */
let latest: Snapshot | null = null;
const listeners = new Set<() => void>();

export function publishSnapshot(s: Snapshot): void {
  latest = s;
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export function useLatestSnapshot(): Snapshot | null {
  return useSyncExternalStore(subscribe, () => latest);
}

/** For tests. */
export function resetLiveStore(): void {
  latest = null;
}
