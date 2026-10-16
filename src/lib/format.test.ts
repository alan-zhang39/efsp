import { describe, expect, it } from "vitest";
import { formatDate, formatTime } from "./format";

describe("dates and times", () => {
  it("shows month and day, and the year only when it isn't this year", () => {
    const now = new Date(2026, 9, 13);
    const thisYear = formatDate(new Date(2026, 9, 3, 18, 5).getTime() / 1000, now);
    expect(thisYear).toMatch(/3/);
    expect(thisYear).not.toMatch(/2026/);
    expect(formatDate(new Date(2025, 0, 2).getTime() / 1000, now)).toMatch(/2025/);
  });

  it("shows hours and minutes", () => {
    expect(formatTime(new Date(2026, 9, 13, 21, 53).getTime() / 1000)).toMatch(/53/);
  });
});
