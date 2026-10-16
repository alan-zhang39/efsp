import { invoke } from "@tauri-apps/api/core";

/** Mirrors `settings::Settings` in src-tauri/src/settings.rs. */
export interface Show {
  jitter: boolean;
  loss: boolean;
  packets: boolean;
  graphJitter: boolean;
  graphPackets: boolean;
  graphLosses: boolean;
}

export type Theme = "dark" | "light" | "system";
export type HistoryView = "rows" | "cards";
/** The Live card: details beside the ping (card) or one big centred ping (focus). */
export type LiveLayout = "card" | "focus";

export interface Settings {
  theme: Theme;
  startWithWindows: boolean;
  startMinimised: boolean;
  show: Show;
  historyView: HistoryView;
  /** Rows or cards on the Servers page. */
  serversView: HistoryView;
  liveLayout: LiveLayout;
  /** null = keep forever. */
  keepHistoryDays: number | null;
}

export const KEEP_DAYS_OPTIONS: readonly (number | null)[] = [null, 30, 90, 365];

export const DEFAULT_SETTINGS: Settings = {
  theme: "dark",
  startWithWindows: false,
  startMinimised: false,
  show: {
    jitter: true,
    loss: true,
    packets: false,
    graphJitter: true,
    graphPackets: false,
    graphLosses: true,
  },
  historyView: "rows",
  serversView: "rows",
  liveLayout: "card",
  keepHistoryDays: null,
};

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null;

function pickBool(o: Obj, key: string, fallback: boolean): boolean {
  const v = o[key];
  return typeof v === "boolean" ? v : fallback;
}

/**
 * Tolerant parse: anything missing or odd falls back to the default, so a
 * settings problem never stops the app from showing the card.
 */
export function parseSettings(v: unknown): Settings {
  if (!isObj(v)) return DEFAULT_SETTINGS;
  const d = DEFAULT_SETTINGS;
  const show = isObj(v.show) ? v.show : {};
  const theme =
    v.theme === "light" || v.theme === "system" || v.theme === "dark" ? v.theme : d.theme;
  const keep = v.keepHistoryDays;
  return {
    theme,
    startWithWindows: pickBool(v, "startWithWindows", d.startWithWindows),
    startMinimised: pickBool(v, "startMinimised", d.startMinimised),
    show: {
      jitter: pickBool(show, "jitter", d.show.jitter),
      loss: pickBool(show, "loss", d.show.loss),
      packets: pickBool(show, "packets", d.show.packets),
      graphJitter: pickBool(show, "graphJitter", d.show.graphJitter),
      graphPackets: pickBool(show, "graphPackets", d.show.graphPackets),
      graphLosses: pickBool(show, "graphLosses", d.show.graphLosses),
    },
    historyView: v.historyView === "cards" ? "cards" : "rows",
    serversView: v.serversView === "cards" ? "cards" : "rows",
    liveLayout: v.liveLayout === "focus" ? "focus" : "card",
    keepHistoryDays:
      typeof keep === "number" && KEEP_DAYS_OPTIONS.includes(keep) ? keep : d.keepHistoryDays,
  };
}

export async function getSettings(): Promise<Settings> {
  return parseSettings(await invoke<unknown>("settings_get"));
}

/** Saves and applies; resolves with what was saved. */
export async function saveSettings(s: Settings): Promise<Settings> {
  return parseSettings(await invoke<unknown>("settings_set", { settings: s }));
}
