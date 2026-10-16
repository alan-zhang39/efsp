import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, vi } from "vitest";

// uPlot needs a real canvas and matchMedia, which jsdom lacks. Every test
// gets this stand-in; it records what the graph was given.
vi.mock("../components/PingGraph", () => ({
  PingGraph: ({ points, live }: { points: unknown[]; live: boolean }) =>
    createElement("div", {
      "data-testid": "graph",
      "data-live": String(live),
      "data-points": points.length,
    }),
}));

// Base UI's popups, switches and toggles use browser APIs jsdom lacks.
// Minimal stand-ins, so a failing test means behaviour changed, not that
// the test environment is missing something.
const noop = (): undefined => undefined;
/** Defines `name` on `target` only if jsdom doesn't have it. */
function standIn(target: object, name: string, value: unknown): void {
  if (!(name in target)) Object.defineProperty(target, name, { value, writable: true });
}
standIn(
  globalThis,
  "ResizeObserver",
  class {
    observe = noop;
    unobserve = noop;
    disconnect = noop;
  },
);
standIn(
  window,
  "PointerEvent",
  class extends MouseEvent {
    readonly pointerId: number;
    readonly pointerType: string;
    constructor(type: string, init: PointerEventInit = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 1;
      this.pointerType = init.pointerType ?? "mouse";
    }
  },
);
standIn(Element.prototype, "scrollIntoView", noop);
standIn(Element.prototype, "hasPointerCapture", () => false);
standIn(Element.prototype, "setPointerCapture", noop);
standIn(Element.prototype, "releasePointerCapture", noop);
standIn(window, "matchMedia", (query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: noop,
  removeListener: noop,
  addEventListener: noop,
  removeEventListener: noop,
  dispatchEvent: () => false,
}));

afterEach(() => {
  cleanup();
});
