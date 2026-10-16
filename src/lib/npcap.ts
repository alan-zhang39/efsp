import { invoke } from "@tauri-apps/api/core";

/** Mirrors `efsp_core::npcap::NpcapStatus` (crates/core/src/npcap.rs). */
export type NpcapStatus =
  | { state: "found"; version: string | null }
  | { state: "restricted"; version: string | null }
  | { state: "missing"; partial: boolean }
  | { state: "notApplicable" };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/**
 * Validates what came back over IPC instead of trusting a type cast, so a
 * mismatch between Rust and TS fails loudly rather than rendering nonsense.
 */
export function parseNpcapStatus(value: unknown): NpcapStatus {
  if (isRecord(value)) {
    const { state } = value;
    if (state === "found" || state === "restricted") {
      const { version } = value;
      if (typeof version === "string" || version === null) return { state, version };
    }
    if (state === "missing" && typeof value.partial === "boolean") {
      return { state, partial: value.partial };
    }
    if (state === "notApplicable") return { state };
  }
  throw new Error(`Unexpected Npcap status from the app: ${JSON.stringify(value)}`);
}

export async function getNpcapStatus(): Promise<NpcapStatus> {
  return parseNpcapStatus(await invoke<unknown>("npcap_status"));
}

/** Opens the official Npcap download page in the default browser. */
export async function openNpcapDownload(): Promise<void> {
  await invoke("open_npcap_download");
}
