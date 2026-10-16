import { describe, expect, it } from "vitest";
import { formatDuration, tierFor } from "./bars";

describe("bars helpers", () => {
  it("maps bars to colour tiers", () => {
    expect([5, 4, 3, 2, 1].map(tierFor)).toEqual(["high", "high", "mid", "low", "low"]);
  });

  it("formats match durations", () => {
    expect(formatDuration(45_000)).toBe("45 s");
    expect(formatDuration(80_000)).toBe("1 min 20 s");
    expect(formatDuration(120_000)).toBe("2 min");
    expect(formatDuration(725_000)).toBe("12 min");
  });
});
