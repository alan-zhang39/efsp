import { useCallback, useEffect, useState } from "react";
import { getNpcapStatus, type NpcapStatus } from "./npcap";

export type NpcapResult =
  { kind: "status"; status: NpcapStatus } | { kind: "error"; message: string };

/**
 * Loads the Npcap status on mount and on `recheck()`. While re-checking, the
 * previous result stays on screen so the layout doesn't jump.
 */
export function useNpcapStatus(): {
  result: NpcapResult | null;
  checking: boolean;
  recheck: () => void;
} {
  const [result, setResult] = useState<NpcapResult | null>(null);
  const [checking, setChecking] = useState(true);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    getNpcapStatus().then(
      (status) => {
        if (cancelled) return;
        setResult({ kind: "status", status });
        setChecking(false);
      },
      (error: unknown) => {
        if (cancelled) return;
        setResult({
          kind: "error",
          message: error instanceof Error ? error.message : String(error),
        });
        setChecking(false);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [attempt]);

  const recheck = useCallback(() => {
    setChecking(true);
    setAttempt((n) => n + 1);
  }, []);

  return { result, checking, recheck };
}
