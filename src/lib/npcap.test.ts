import { describe, expect, it } from "vitest";
import { parseNpcapStatus } from "./npcap";

describe("parseNpcapStatus", () => {
  it("accepts every shape the Rust side sends", () => {
    expect(parseNpcapStatus({ state: "found", version: "1.80" })).toEqual({
      state: "found",
      version: "1.80",
    });
    expect(parseNpcapStatus({ state: "found", version: null })).toEqual({
      state: "found",
      version: null,
    });
    expect(parseNpcapStatus({ state: "restricted", version: null })).toEqual({
      state: "restricted",
      version: null,
    });
    expect(parseNpcapStatus({ state: "missing", partial: true })).toEqual({
      state: "missing",
      partial: true,
    });
    expect(parseNpcapStatus({ state: "notApplicable" })).toEqual({ state: "notApplicable" });
  });

  it("rejects anything else", () => {
    for (const bad of [
      null,
      "found",
      {},
      { state: "found" },
      { state: "missing" },
      { state: "x" },
    ]) {
      expect(() => parseNpcapStatus(bad)).toThrow(/Unexpected Npcap status/);
    }
  });
});
