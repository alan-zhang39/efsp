import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { DEFAULT_SETTINGS, getSettings, saveSettings, type Settings } from "./settings";

interface SettingsState {
  settings: Settings;
  /** The theme actually in use (system resolved to dark or light). */
  resolvedTheme: "dark" | "light";
  update: (next: Settings) => Promise<void>;
  error: string | null;
}

const Ctx = createContext<SettingsState | null>(null);

function systemTheme(): "dark" | "light" {
  return typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [system, setSystem] = useState<"dark" | "light">(systemTheme);
  const [error, setError] = useState<string | null>(null);

  // Follow the OS while the theme is "system".
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => {
      setSystem(mq.matches ? "light" : "dark");
    };
    mq.addEventListener("change", onChange);
    return () => {
      mq.removeEventListener("change", onChange);
    };
  }, []);

  const resolvedTheme = settings.theme === "system" ? system : settings.theme;

  // A layout effect runs before children's ordinary effects, so graphs that
  // redraw on a theme change read the NEW colours (a plain effect here would
  // run after them and they'd keep the old palette).
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = resolvedTheme;
    document.querySelector('meta[name="color-scheme"]')?.setAttribute("content", resolvedTheme);
  }, [resolvedTheme]);

  // The last settings the app confirmed, to roll back to if a save fails.
  const confirmed = useRef<Settings>(DEFAULT_SETTINGS);
  useEffect(() => {
    getSettings().then(
      (s) => {
        confirmed.current = s;
        setSettings(s);
      },
      () => {
        // Defaults stay in place; the app still works.
      },
    );
  }, []);

  const update = useCallback(async (next: Settings) => {
    setError(null);
    setSettings(next); // optimistic: the UI responds at once
    try {
      // The app may return the previous settings (e.g. a cancelled
      // "shorten history" confirmation): show whatever is really in effect.
      const saved = await saveSettings(next);
      confirmed.current = saved;
      setSettings(saved);
    } catch (e) {
      setSettings(confirmed.current);
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  return <Ctx.Provider value={{ settings, resolvedTheme, update, error }}>{children}</Ctx.Provider>;
}

export function useSettings(): SettingsState {
  const v = useContext(Ctx);
  if (v === null) throw new Error("useSettings must be used inside SettingsProvider");
  return v;
}
