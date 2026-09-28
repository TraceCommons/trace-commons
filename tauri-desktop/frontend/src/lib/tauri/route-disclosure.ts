// Kept free of imports so the node test runner can load it directly.

/**
 * K11: what leaves this machine, to whom, and what this client checked.
 *
 * `facts` is the daemon's `route_disclosure` answer; `copy` is the
 * contributor core's words for exactly those facts
 * (`consent_copy::route_disclosure_copy`). This shell renders both and
 * chooses neither. The parser refuses, rather than renders, a payload whose
 * words do not match its facts, or a route or origin this build does not
 * know: the nearest known value would claim something the daemon did not.
 */

export type Route =
  | "witness"
  | "witness_refusing"
  | "local"
  | "not_enrolled"
  | "settings_unreadable";

export type WitnessOrigin =
  | "published_at_join"
  | "connected_inference"
  | "settings"
  | "environment"
  | "not_recorded";

export type LocalFilter =
  | "none"
  | "near_ai"
  | "self_hosted"
  | "sidecar"
  | "invalid";

export type WitnessFacts = {
  state: string;
  url: string;
  signing_address: string;
  pinned_measurements: string[];
  origin: WitnessOrigin;
};

export type RouteFacts = {
  route: Route;
  witness: WitnessFacts | null;
  local_filter: LocalFilter | null;
  receipts: { endpoint_configured: boolean; check_attestation: boolean };
  attested_bodies: boolean;
};

export type SessionSendCopy = {
  heading: string;
  before_label: string;
  before_line: string;
  after_label: string;
  after_line: string;
};

export type WitnessDisclosureCopy = {
  heading: string;
  address_label: string;
  signing_label: string;
  measurements_label: string;
  check: string;
  classifier: string | null;
  origin: string;
};

export type RouteDisclosureCopy = {
  title: string;
  route: string;
  witness: WitnessDisclosureCopy | null;
  local_filter: string | null;
  receipts: string | null;
  attested_bodies: string | null;
  session: SessionSendCopy;
};

export type RouteDisclosure = { facts: RouteFacts; copy: RouteDisclosureCopy };

export type CertificateDetail = {
  detail: {
    witness_measurement: string;
    signer: string;
    verification: "verified_at_review";
  };
  copy: {
    heading: string;
    measurement_label: string;
    signer_label: string;
    verified_at_review: string;
  };
};

type RecordValue = Record<string, unknown>;

const ROUTES: readonly string[] = [
  "witness",
  "witness_refusing",
  "local",
  "not_enrolled",
  "settings_unreadable",
];
const ORIGINS: readonly string[] = [
  "published_at_join",
  "connected_inference",
  "settings",
  "environment",
  "not_recorded",
];
const FILTERS: readonly string[] = [
  "none",
  "near_ai",
  "self_hosted",
  "sidecar",
  "invalid",
];

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid ${label}`);
  }
  return value as RecordValue;
}

function text(value: RecordValue, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid route disclosure field: ${key}`);
  }
  return field;
}

function optionalText(value: RecordValue, key: string): string | null {
  const field = value[key];
  if (field === null || field === undefined) return null;
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid route disclosure field: ${key}`);
  }
  return field;
}

function oneOf(value: unknown, allowed: readonly string[], label: string) {
  if (typeof value !== "string" || !allowed.includes(value)) {
    throw new Error(`Unknown ${label}`);
  }
  return value;
}

function bool(value: RecordValue, key: string): boolean {
  if (typeof value[key] !== "boolean") {
    throw new Error(`Invalid route disclosure field: ${key}`);
  }
  return value[key] as boolean;
}

function witnessFacts(value: unknown): WitnessFacts | null {
  if (value === null || value === undefined) return null;
  const item = record(value, "witness facts");
  const pins = item.pinned_measurements;
  if (!Array.isArray(pins) || pins.some((pin) => typeof pin !== "string")) {
    throw new Error("Invalid route disclosure field: pinned_measurements");
  }
  return {
    state: text(item, "state"),
    url: text(item, "url"),
    signing_address: text(item, "signing_address"),
    pinned_measurements: pins as string[],
    origin: oneOf(item.origin, ORIGINS, "witness origin") as WitnessOrigin,
  };
}

function facts(value: unknown): RouteFacts {
  const item = record(value, "route disclosure facts");
  const receipts = record(item.receipts, "receipt facts");
  const filter = item.local_filter;
  return {
    route: oneOf(item.route, ROUTES, "route") as Route,
    witness: witnessFacts(item.witness),
    local_filter:
      filter === null || filter === undefined
        ? null
        : (oneOf(filter, FILTERS, "local filter") as LocalFilter),
    receipts: {
      endpoint_configured: bool(receipts, "endpoint_configured"),
      check_attestation: bool(receipts, "check_attestation"),
    },
    attested_bodies: bool(item, "attested_bodies"),
  };
}

function witnessCopy(value: unknown): WitnessDisclosureCopy | null {
  if (value === null || value === undefined) return null;
  const item = record(value, "witness copy");
  return {
    heading: text(item, "heading"),
    address_label: text(item, "address_label"),
    signing_label: text(item, "signing_label"),
    measurements_label: text(item, "measurements_label"),
    check: text(item, "check"),
    classifier: optionalText(item, "classifier"),
    origin: text(item, "origin"),
  };
}

function sessionCopy(value: unknown): SessionSendCopy {
  const item = record(value, "session copy");
  return {
    heading: text(item, "heading"),
    before_label: text(item, "before_label"),
    before_line: text(item, "before_line"),
    after_label: text(item, "after_label"),
    after_line: text(item, "after_line"),
  };
}

/** Throws unless the words are exactly the blocks the facts allow. */
function requireCrossing(f: RouteFacts, c: RouteDisclosureCopy) {
  const sendsToWitness = f.route === "witness";
  if ((f.witness === null) !== (c.witness === null)) {
    throw new Error("Route disclosure witness words do not match its facts");
  }
  if (c.witness && (c.witness.classifier !== null) !== sendsToWitness) {
    throw new Error("Route disclosure classifier line on the wrong route");
  }
  if ((c.local_filter !== null) !== (f.route === "local")) {
    throw new Error("Route disclosure local filter line on the wrong route");
  }
  if ((c.receipts !== null) !== sendsToWitness) {
    throw new Error("Route disclosure receipt line on the wrong route");
  }
  if ((c.attested_bodies !== null) !== (sendsToWitness && f.attested_bodies)) {
    throw new Error("Route disclosure attested-bodies line on the wrong route");
  }
}

export function parseRouteDisclosure(value: unknown): RouteDisclosure {
  const item = record(value, "route disclosure");
  const f = facts(item.facts);
  const raw = record(item.copy, "route disclosure copy");
  const c: RouteDisclosureCopy = {
    title: text(raw, "title"),
    route: text(raw, "route"),
    witness: witnessCopy(raw.witness),
    local_filter: optionalText(raw, "local_filter"),
    receipts: optionalText(raw, "receipts"),
    attested_bodies: optionalText(raw, "attested_bodies"),
    session: sessionCopy(raw.session),
  };
  requireCrossing(f, c);
  return { facts: f, copy: c };
}

/**
 * The daemon's `certificate_detail` for one entry, with the core's labels.
 * Only `verified_at_review` is worded: the claim is about the review, and a
 * verification this build has no sentence for is not rendered as that one.
 */
export function parseCertificateDetail(value: unknown): CertificateDetail {
  const item = record(value, "certificate detail");
  const detail = record(item.detail, "certificate detail claims");
  if (detail.verification !== "verified_at_review") {
    throw new Error("Unknown certificate verification");
  }
  const copy = record(item.copy, "certificate detail copy");
  return {
    detail: {
      witness_measurement: text(detail, "witness_measurement"),
      signer: text(detail, "signer"),
      verification: "verified_at_review",
    },
    copy: {
      heading: text(copy, "heading"),
      measurement_label: text(copy, "measurement_label"),
      signer_label: text(copy, "signer_label"),
      verified_at_review: text(copy, "verified_at_review"),
    },
  };
}
