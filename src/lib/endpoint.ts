/** Mirrors `efsp_core::classify`. No addresses, by design. */

export type Continent =
  "europe" | "northAmerica" | "southAmerica" | "asia" | "oceania" | "africa" | "middleEast";

export interface Place {
  city: string;
  /** ISO 3166-1 alpha-2. */
  country: string;
  continent: Continent;
}

export type Endpoint =
  | { kind: "server"; region: string; place: Place | null }
  | { kind: "peer"; country: string | null }
  | { kind: "local" }
  /** network/country are set only for known hosting operators. */
  | { kind: "unknown"; network: string | null; country: string | null };

type Obj = Record<string, unknown>;

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null;
}

function fail(what: string, v: unknown): never {
  throw new Error(`Unexpected ${what} from the app: ${JSON.stringify(v)}`);
}

function strOrNull(o: Obj, key: string): string | null {
  const v = o[key];
  if (v === null) return null;
  return typeof v === "string" ? v : fail(key, v);
}

function str(o: Obj, key: string): string {
  const v = o[key];
  return typeof v === "string" ? v : fail(key, v);
}

const CONTINENTS: readonly Continent[] = [
  "europe",
  "northAmerica",
  "southAmerica",
  "asia",
  "oceania",
  "africa",
  "middleEast",
];

function parsePlace(v: unknown): Place | null {
  if (v === null) return null;
  if (!isObj(v)) fail("place", v);
  const continent = CONTINENTS.find((c) => c === v.continent) ?? fail("continent", v.continent);
  return { city: str(v, "city"), country: str(v, "country"), continent };
}

export function parseEndpoint(v: unknown): Endpoint {
  if (!isObj(v)) fail("endpoint", v);
  switch (v.kind) {
    case "server": {
      return { kind: "server", region: str(v, "region"), place: parsePlace(v.place) };
    }
    case "peer":
      return { kind: "peer", country: strOrNull(v, "country") };
    case "local":
      return { kind: "local" };
    case "unknown":
      return {
        kind: "unknown",
        network: strOrNull(v, "network"),
        country: strOrNull(v, "country"),
      };
    default:
      return fail("endpoint kind", v.kind);
  }
}

export function parseEndpointOrNull(v: unknown): Endpoint | null {
  return v === null ? null : parseEndpoint(v);
}

let regionNames: Intl.DisplayNames | null = null;

/** "DE" → "Germany", in the user's language; the code if unknown. */
export function countryName(code: string): string {
  try {
    regionNames ??= new Intl.DisplayNames(undefined, { type: "region" });
    return regionNames.of(code) ?? code;
  } catch {
    return code;
  }
}

/** "Frankfurt", "Canada (peer)", "Local network", "Unclassified". */
export function endpointLabel(e: Endpoint): string {
  switch (e.kind) {
    case "server":
      return e.place === null ? e.region : e.place.city;
    case "peer":
      return e.country === null ? "Peer" : `${countryName(e.country)} (peer)`;
    case "local":
      return "Local network";
    case "unknown":
      if (e.network === null) return "Unclassified";
      return e.country === null
        ? `Unclassified · ${e.network}`
        : `Unclassified · ${e.network}, ${countryName(e.country)}`;
  }
}

/** Stable key for an endpoint (e.g. a list row): the city for servers. */
export function endpointKey(e: Endpoint): string {
  switch (e.kind) {
    // By city, so every region there is one place (matches Rust's Endpoint::area_key).
    case "server":
      return e.place === null ? `server:${e.region}` : `server:${e.place.city}|${e.place.country}`;
    case "peer":
      return `peer:${e.country ?? ""}`;
    case "local":
      return "local";
    case "unknown":
      return `unknown:${e.network ?? ""}:${e.country ?? ""}`;
  }
}

/** Short place for lists: the same as {@link endpointLabel}. */
export const placeName = endpointLabel;
