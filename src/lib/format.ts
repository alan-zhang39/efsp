const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

/** 1234 → "1.2K". */
export function formatCount(n: number): string {
  return compact.format(n);
}

/** Milliseconds for display: "34 ms", "4.5 ms" (one decimal under 10). */
export function formatMs(ms: number): string {
  return ms < 10 ? `${ms.toFixed(1)} ms` : `${String(Math.round(ms))} ms`;
}

/** Milliseconds → "45 s", "3 min", "1 h 5 min". */
export function formatAge(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${String(s)} s`;
  const min = Math.floor(s / 60);
  if (min < 60) return `${String(min)} min`;
  const h = Math.floor(min / 60);
  const rest = min % 60;
  return rest === 0 ? `${String(h)} h` : `${String(h)} h ${String(rest)} min`;
}

/** "Oct 13" or "13 Oct", following the PC's region; the year only if not this one. */
export function formatDate(unixSeconds: number, now: Date = new Date()): string {
  const d = new Date(unixSeconds * 1000);
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

/** "21:53" or "9:53 PM", following the PC's region. */
export function formatTime(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}
