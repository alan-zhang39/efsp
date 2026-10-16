import { cn } from "cn";
import type { ReactNode } from "react";

/**
 * Every page's frame: the same top, the same gap between sections, and a
 * header with the title and any actions at the same height everywhere.
 */
export function Page({
  className,
  children,
}: {
  className?: string | undefined;
  children: ReactNode;
}) {
  return (
    <div data-slot="page" className={cn("grid content-start gap-6 *:min-w-0", className)}>
      {children}
    </div>
  );
}

export function PageHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex min-h-11 items-center justify-between gap-4">
      <h1 className="m-0 text-2xl/normal font-[650] tracking-[-0.015em]">{title}</h1>
      {children !== undefined && <div className="flex items-center gap-2">{children}</div>}
    </div>
  );
}

/** A quiet note under a control, e.g. "Saved as CSV." */
export function Hint({ children }: { children: ReactNode }) {
  return (
    <p className="m-0 text-[0.8125rem] text-muted-foreground" role="status">
      {children}
    </p>
  );
}

/** An inline problem that needs attention (in the warning colour). */
export function InlineError({
  className,
  children,
}: {
  className?: string | undefined;
  children: ReactNode;
}) {
  return (
    <p className={cn("m-0 text-sm text-action", className)} role="alert">
      {children}
    </p>
  );
}

/** The row of filters under a page's header. */
export function FilterBar({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center gap-x-4 gap-y-3">{children}</div>;
}

/** What's shown when a list is empty. */
export function Empty({ children }: { children: ReactNode }) {
  return <p className="m-0 text-[0.9375rem] text-muted-foreground">{children}</p>;
}
