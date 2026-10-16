import { useEffect, useState } from "react";
import { publishSnapshot } from "./liveStore";
import { startMonitor, type Snapshot } from "./monitor";

export function useMonitor(): { snapshot: Snapshot | null; error: string | null } {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let cancelled = false;
    const onSnapshot = (s: Snapshot) => {
      // A good snapshot after a bad one: the problem is over.
      setError(null);
      setSnapshot(s);
      publishSnapshot(s);
    };
    startMonitor(onSnapshot, setError).then(
      (s) => {
        if (cancelled) s();
        else stop = s;
      },
      (e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      },
    );
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  return { snapshot, error };
}
