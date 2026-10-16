import { invoke } from "@tauri-apps/api/core";
import { useEffect, useSyncExternalStore } from "react";

/** A city to switch off (mirrors `efsp_core::picker::Area`). */
export interface Area {
  /** `endpointKey` of the city. */
  key: string;
  name: string;
}

/** Mirrors `PickerStatus` in src-tauri/src/lib.rs. */
export interface PickerStatus {
  off: Area[];
  /** efsp's rules are in Windows Firewall right now. */
  rulesOn: boolean;
  gameExe: string | null;
  /** This build can change the firewall (Windows with the helper). */
  available: boolean;
  matchActive: boolean;
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null;
function fail(what: string, v: unknown): never {
  throw new Error(`Unexpected ${what} from the app: ${JSON.stringify(v)}`);
}
function bool(o: Obj, k: string): boolean {
  const v = o[k];
  return typeof v === "boolean" ? v : fail(k, v);
}

export function parseStatus(v: unknown): PickerStatus {
  if (!isObj(v) || !Array.isArray(v.off)) fail("picker status", v);
  const gameExe = v.gameExe;
  if (gameExe !== null && typeof gameExe !== "string") fail("gameExe", gameExe);
  return {
    off: v.off.map((a: unknown) => {
      if (!isObj(a) || typeof a.key !== "string" || typeof a.name !== "string") fail("area", a);
      return { key: a.key, name: a.name };
    }),
    rulesOn: bool(v, "rulesOn"),
    gameExe,
    available: bool(v, "available"),
    matchActive: bool(v, "matchActive"),
  };
}

// One status shared by the sidebar and the Servers page.
let current: PickerStatus | null = null;
const listeners = new Set<() => void>();
function publish(s: PickerStatus): void {
  current = s;
  for (const l of listeners) l();
}

export async function refreshPicker(): Promise<void> {
  publish(parseStatus(await invoke<unknown>("picker_status")));
}

/** Runs a command that returns a new status, or null when cancelled. */
async function change(cmd: string, args?: Record<string, unknown>): Promise<boolean> {
  const v = await invoke<unknown>(cmd, args);
  if (v === null) return false;
  publish(parseStatus(v));
  return true;
}

/** Switches these cities off (one Windows prompt). False if cancelled. */
export function applyPicker(off: Area[]): Promise<boolean> {
  return change("picker_apply", { off });
}

/** Turns every city back on (removes efsp's rules). False if cancelled. */
export function removeAllRules(): Promise<boolean> {
  return change("picker_remove_all");
}

export function chooseGameExe(): Promise<boolean> {
  return change("picker_choose_exe");
}

export function usePicker(): PickerStatus | null {
  const status = useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => current,
  );
  useEffect(() => {
    if (current === null) void refreshPicker().catch(() => undefined);
  }, []);
  return status;
}

/** For tests. */
export function resetPicker(): void {
  current = null;
}
