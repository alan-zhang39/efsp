import { cn } from "cn";
import type { ReactNode } from "react";

export type Tone = "ready" | "action" | "neutral";

const LABEL: Record<Tone, string> = {
  ready: "text-ready",
  action: "text-action",
  neutral: "text-neutral",
};

const MARK: Record<Tone, string> = {
  ready: "bg-ready text-background",
  action: "border-[1.5px] border-action",
  neutral: "",
};

/**
 * A full-page status: a small label with a mark, a large heading and an
 * optional line of text, then any actions. It grows with the window, so a
 * big window isn't mostly empty. Key it by heading so each new state
 * settles in again.
 */
export function Status({
  tone,
  mark,
  label,
  heading,
  body,
  children,
}: {
  tone: Tone;
  mark: string;
  label: string;
  heading: string;
  body?: string | null | undefined;
  children?: ReactNode;
}) {
  return (
    <section className="max-w-[clamp(34rem,55vw,48rem)] animate-settle motion-reduce:animate-fade">
      <p
        className={cn(
          "m-0 mb-3 flex items-center gap-2 text-[clamp(0.9375rem,1.4vw,1.125rem)] font-semibold",
          LABEL[tone],
        )}
      >
        <span
          className={cn(
            "inline-grid size-[1.375rem] place-items-center rounded-full text-[0.8125rem] leading-none",
            MARK[tone],
          )}
          aria-hidden="true"
        >
          {mark}
        </span>
        {label}
      </p>
      <h1 className="m-0 mb-3 text-[clamp(1.75rem,4.5vw,3.5rem)] leading-[1.1] font-[650] tracking-[-0.022em] text-balance">
        {heading}
      </h1>
      {body != null && (
        <p className="m-0 max-w-[34em] text-[clamp(1rem,1.5vw,1.25rem)] leading-[1.55] text-pretty text-muted-foreground">
          {body}
        </p>
      )}
      {children}
    </section>
  );
}

/** The buttons under a status. */
export function StatusActions({ children }: { children: ReactNode }) {
  return <div className="mt-6 flex flex-wrap gap-3">{children}</div>;
}
