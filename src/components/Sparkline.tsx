/**
 * Tiny ping line for a history card (24 points). Decorative: the ping
 * number carries the meaning; this only hints at how steady the match was.
 */
export function Sparkline({ values }: { values: (number | null)[] }) {
  const known = values.filter((v): v is number => v !== null);
  if (known.length < 2) return <span className="inline-block w-24" aria-hidden="true" />;
  const max = Math.max(...known);
  const min = Math.min(...known);
  const span = Math.max(max - min, 10); // flat-ish matches stay flat
  const w = 96;
  const h = 24;
  const step = w / Math.max(values.length - 1, 1);
  let d = "";
  let pen = false;
  values.forEach((v, i) => {
    if (v === null) {
      pen = false;
      return;
    }
    const x = i * step;
    const y = h - 2 - ((v - min) / span) * (h - 4);
    d += `${pen ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
    pen = true;
  });
  return (
    <svg className="h-6 w-24" viewBox={`0 0 ${String(w)} ${String(h)}`} aria-hidden="true">
      <path
        d={d}
        className="fill-none stroke-muted-foreground"
        strokeWidth={1.5}
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
