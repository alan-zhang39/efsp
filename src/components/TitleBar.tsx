import { getCurrentWindow, type Window } from "@tauri-apps/api/window";
import { cn } from "cn";
import { useEffect, useState } from "react";

/** The Tauri window, or null outside the app (tests, browser preview). */
function appWindow(): Window | null {
  try {
    return "__TAURI_INTERNALS__" in window ? getCurrentWindow() : null;
  } catch {
    return null;
  }
}

/** Windows-style caption buttons; close turns red on hover, as on Windows. */
const WINDOW_BUTTON =
  "grid w-[2.875rem] cursor-default place-items-center border-0 bg-transparent text-muted-foreground hover:bg-surface hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring";

const icon = {
  width: 10,
  height: 10,
  viewBox: "0 0 10 10",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1,
  "aria-hidden": true,
};

/**
 * efsp's own title bar (the window has no native frame): the app's name at
 * the left, a drag area, and minimise / maximise / close at the top right. Double-click the bar to
 * maximise, as with a normal Windows title bar.
 */
export function TitleBar() {
  const [win] = useState(appWindow);
  const [maximised, setMaximised] = useState(false);

  // Keep the maximise/restore icon right however the size changed (button,
  // double-click, Win+Up, snapping). A DOM resize needs no extra permission.
  useEffect(() => {
    if (win === null) return;
    const check = () => {
      win.isMaximized().then(setMaximised, () => undefined);
    };
    check();
    window.addEventListener("resize", check);
    return () => {
      window.removeEventListener("resize", check);
    };
  }, [win]);

  return (
    <header className="flex items-stretch justify-between select-none" data-tauri-drag-region>
      {/* Children don't inherit the drag region, so the name carries it too. */}
      <p
        className="m-0 flex cursor-default items-center pl-3.5 text-[0.8125rem] font-semibold tracking-[0.01em] text-muted-foreground"
        data-tauri-drag-region
      >
        efsp
      </p>
      {win !== null && (
        <div className="flex">
          <button
            type="button"
            className={WINDOW_BUTTON}
            aria-label="Minimise"
            onClick={() => void win.minimize()}
          >
            <svg {...icon}>
              <path d="M0 5.5h10" />
            </svg>
          </button>
          <button
            type="button"
            className={WINDOW_BUTTON}
            aria-label={maximised ? "Restore" : "Maximise"}
            onClick={() => void win.toggleMaximize()}
          >
            {maximised ? (
              <svg {...icon}>
                <path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" />
              </svg>
            ) : (
              <svg {...icon}>
                <rect x=".5" y=".5" width="9" height="9" />
              </svg>
            )}
          </button>
          <button
            type="button"
            className={cn(WINDOW_BUTTON, "hover:bg-[#c42b1c] hover:text-white")}
            aria-label="Close"
            onClick={() => void win.close()}
          >
            <svg {...icon}>
              <path d="M.5.5l9 9M9.5.5l-9 9" />
            </svg>
          </button>
        </div>
      )}
    </header>
  );
}
