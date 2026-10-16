import { cn } from "cn";
import type { ReactNode } from "react";

/** A small outlined tag beside a name, e.g. "Offer" or "TCP". */
function Badge({ className, children }: { className?: string | undefined; children: ReactNode }) {
  return (
    <span
      data-slot="badge"
      className={cn(
        "ml-2 inline-block rounded-sm border border-line px-1.5 py-px align-[0.1em] text-[0.6875rem] font-semibold text-muted-foreground",
        className,
      )}
    >
      {children}
    </span>
  );
}

export { Badge };
