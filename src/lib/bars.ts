/**
 * Colour tier for efsp's bars. Internal names for styling only:
 * the UI never shows verdict words, and the ping number and bar count carry
 * the meaning, so colour is never the only signal.
 */
export type Tier = "high" | "mid" | "low";

export function tierFor(bars: number): Tier {
  if (bars >= 4) return "high";
  if (bars === 3) return "mid";
  return "low";
}

/** "1 min 20 s", "12 min", "45 s". */
export function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000);
  const min = Math.floor(total / 60);
  const s = total % 60;
  if (min === 0) return `${String(s)} s`;
  return s === 0 || min >= 10 ? `${String(min)} min` : `${String(min)} min ${String(s)} s`;
}
