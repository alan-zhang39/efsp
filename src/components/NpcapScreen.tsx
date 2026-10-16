import { useState } from "react";
import { Button } from "@/components/ui/button";
import { openNpcapDownload } from "../lib/npcap";
import type { NpcapResult } from "../lib/useNpcapStatus";
import { InlineError } from "./Page";
import { Status, StatusActions, type Tone } from "./Status";

interface View {
  tone: Tone;
  /** Shape differs per tone so status never relies on colour alone. */
  mark: string;
  label: string;
  heading: string;
  body: string;
  offerDownload: boolean;
}

function describe(result: NpcapResult | null): View {
  if (result === null) {
    return {
      tone: "neutral",
      mark: "◌",
      label: "Checking",
      heading: "Looking for Npcap",
      body: "This takes a moment.",
      offerDownload: false,
    };
  }
  if (result.kind === "error") {
    return {
      tone: "action",
      mark: "!",
      label: "Check failed",
      heading: "Couldn't check for Npcap",
      body: result.message,
      offerDownload: false,
    };
  }
  const { status } = result;
  switch (status.state) {
    case "found":
      return {
        tone: "ready",
        mark: "✓",
        label: "Ready",
        heading: "Npcap is installed",
        body:
          status.version === null
            ? "efsp can use it to time your eFootball connection. Live monitoring arrives in a later build."
            : `Version ${status.version}. efsp can use it to time your eFootball connection. Live monitoring arrives in a later build.`,
        offerDownload: false,
      };
    case "restricted":
      return {
        tone: "action",
        mark: "!",
        label: "Action needed",
        heading: "Npcap is limited to administrators",
        body: "Npcap was installed with “Restrict driver access to Administrators only”, so efsp can't use it. Reinstall Npcap with that option unticked, then check again.",
        offerDownload: true,
      };
    case "missing":
      return status.partial
        ? {
            tone: "action",
            mark: "!",
            label: "Action needed",
            heading: "Npcap needs reinstalling",
            body: "Windows lists Npcap as installed, but its files are missing. Reinstall it from the official site, keeping the default options.",
            offerDownload: true,
          }
        : {
            tone: "action",
            mark: "!",
            label: "Action needed",
            heading: "Install Npcap to continue",
            body: "efsp uses Npcap, the capture driver Wireshark uses, to time your connection. It reads packet timing only, never game data. Install it with the default options, then check again.",
            offerDownload: true,
          };
    case "notApplicable":
      return {
        tone: "neutral",
        mark: "◌",
        label: "Dev mode",
        heading: "Live capture runs on Windows",
        body: "Npcap is a Windows driver, so there is nothing to detect here. On this machine efsp will run from recorded traces.",
        offerDownload: false,
      };
  }
}

interface Props {
  result: NpcapResult | null;
  checking: boolean;
  recheck: () => void;
}

/** Shown until Npcap is usable: checking, missing, restricted, or failed. */
export function NpcapScreen({ result, checking, recheck }: Props) {
  const [openError, setOpenError] = useState<string | null>(null);
  const view = describe(result);
  // Re-checking only helps when something is wrong (after installing, or after an error).
  const showRecheck =
    result?.kind === "error" ||
    (result?.kind === "status" &&
      (result.status.state === "missing" || result.status.state === "restricted"));

  const handleDownload = () => {
    setOpenError(null);
    openNpcapDownload().catch(() => {
      setOpenError("Couldn't open your browser. Go to npcap.com to download Npcap.");
    });
  };

  return (
    <main className="scroll-panel grid h-full min-h-0 grid-rows-[auto_1fr] rounded-[0.625rem] bg-background px-8 pt-6 pb-8 max-[520px]:p-4">
      {/* The live region stays mounted so screen readers announce changes;
          the inner section is keyed so each new state re-runs the settle animation. */}
      <div className="self-center" aria-live="polite" aria-busy={checking}>
        <Status
          key={view.heading}
          tone={view.tone}
          mark={view.mark}
          label={view.label}
          heading={view.heading}
          body={view.body}
        >
          {(view.offerDownload || showRecheck) && (
            <StatusActions>
              {view.offerDownload && (
                <Button variant="primary" onClick={handleDownload}>
                  Get Npcap
                </Button>
              )}
              {showRecheck && (
                <Button onClick={recheck} disabled={checking}>
                  {checking ? "Checking…" : "Check again"}
                </Button>
              )}
            </StatusActions>
          )}
          {view.offerDownload && openError !== null && (
            <InlineError className="mt-4">{openError}</InlineError>
          )}
        </Status>
      </div>
    </main>
  );
}
