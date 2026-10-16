import { describe, expect, it } from "vitest";
import { endpointKey, endpointLabel, parseEndpoint } from "./endpoint";

describe("endpoint", () => {
  it("parses a placed server and labels it by city", () => {
    const e = parseEndpoint({
      kind: "server",
      region: "europe-west3",
      place: { city: "Frankfurt", country: "DE", continent: "europe" },
    });
    expect(endpointLabel(e)).toBe("Frankfurt");
  });

  it("labels an unplaced region by its code", () => {
    const e = parseEndpoint({ kind: "server", region: "us-south-1", place: null });
    expect(endpointLabel(e)).toBe("us-south-1");
  });

  it("names a peer's country, never anything finer", () => {
    expect(endpointLabel(parseEndpoint({ kind: "peer", country: "DE" }))).toBe("Germany (peer)");
    expect(endpointLabel(parseEndpoint({ kind: "peer", country: null }))).toBe("Peer");
  });

  it("labels local and unclassified endpoints", () => {
    expect(endpointLabel(parseEndpoint({ kind: "local" }))).toBe("Local network");
    expect(
      endpointLabel(
        parseEndpoint({ kind: "unknown", network: "Hetzner Online GmbH", country: "DE" }),
      ),
    ).toBe("Unclassified · Hetzner Online GmbH, Germany");
  });

  it("keys servers by city, whichever region code they have", () => {
    const kl = { city: "Kuala Lumpur", country: "MY", continent: "asia" } as const;
    const a = { kind: "server", region: "x1", place: kl } as const;
    const b = { kind: "server", region: "ap-southeast-5", place: kl } as const;
    expect(endpointKey(a)).toBe(endpointKey(b));
    // Without a known city, the region code keeps regions apart.
    expect(endpointKey({ ...a, place: null })).not.toBe(endpointKey({ ...b, place: null }));
  });

  it("rejects unexpected shapes", () => {
    expect(() => parseEndpoint({ kind: "router" })).toThrow();
    expect(() => parseEndpoint({ kind: "server", place: null })).toThrow();
  });
});
