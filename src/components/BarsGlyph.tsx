import { cn } from "cn";
import { formatMs } from "../lib/format";
import { type Tier, tierFor } from "../lib/bars";

/** The text colour for each tier; bars and ping numbers use it. */
export const TIER_TEXT: Record<Tier, string> = {
  high: "text-ready",
  mid: "text-action",
  low: "text-poor",
};

/**
 * efsp's bars: five rising bars, the first `bars` filled. Shape carries the
 * count; colour (by tier) only reinforces it. Empty bars are outlined in the
 * muted text colour so "2 of 5" reads at a glance (3:1 contrast).
 */
export function BarsGlyph({ bars, className }: { bars: number; className?: string }) {
  return (
    <svg
      className={cn("shrink-0", TIER_TEXT[tierFor(bars)], className)}
      viewBox="0 0 50 40"
      role="img"
      aria-label={`${String(bars)} of 5 bars`}
    >
      {[0, 1, 2, 3, 4].map((i) => {
        const h = 12 + i * 7;
        return (
          <rect
            key={i}
            className={
              i < bars ? "fill-current stroke-current" : "fill-none stroke-muted-foreground"
            }
            strokeWidth={1.5}
            x={i * 10 + 1}
            y={40 - h}
            width={7}
            height={h}
            rx={1.5}
          />
        );
      })}
    </svg>
  );
}

/** A ping and its bars side by side, both in the tier's colour. */
export function PingBars({
  ms,
  bars,
  className,
  glyphClassName = "h-[1.4rem] w-7",
}: {
  ms: number;
  bars: number | null;
  className?: string | undefined;
  /** The glyph's size (it doesn't scale with the number by default). */
  glyphClassName?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-end gap-2 font-semibold tabular-nums",
        bars !== null && TIER_TEXT[tierFor(bars)],
        className,
      )}
    >
      {formatMs(ms)}
      {bars !== null && <BarsGlyph bars={bars} className={cn("m-0", glyphClassName)} />}
    </span>
  );
}
