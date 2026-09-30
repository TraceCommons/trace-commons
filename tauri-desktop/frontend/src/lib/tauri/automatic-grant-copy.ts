// Kept free of imports so the node test runner can load it directly.

/** What the fixed patterns remove and where they stop, for one disclosure. */
export type ScrubCopy = { scope: string; limit: string };

export type GrantDisclosure = "patterns_only" | "model_scrubbed";

/**
 * The Flow 1 grant screens' words, from `consent_copy::automatic_grant_copy`.
 *
 * The contributor core decides the disclosure (R1) and sends only the scrub
 * wording that answer allows: exactly one of `patterns_only` and
 * `model_scrubbed` is present. This shell never chooses between them.
 */
export type AutomaticGrantCopy = {
  disclosure: GrantDisclosure;
  scrub: ScrubCopy;
  no_review: string;
  scope_required: string;
  path_automatic: string;
  path_ask_first: string;
  raw_send: string;
};

type RecordValue = Record<string, unknown>;

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`Invalid ${label}`);
  }
  return value as RecordValue;
}

function text(value: RecordValue, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid grant copy field: ${key}`);
  }
  return field;
}

function scrub(value: unknown): ScrubCopy {
  const item = record(value, "grant scrub copy");
  return { scope: text(item, "scope"), limit: text(item, "limit") };
}

/**
 * Refuses, rather than renders, anything but the one shape the core sends.
 * A patterns-only payload that also carried model-scrub wording would be a
 * core bug that could put "a model removed it" in front of a contributor
 * on a route where no model's result is relied on, so it fails closed: no
 * grant screen at all.
 */
export function parseAutomaticGrantCopy(value: unknown): AutomaticGrantCopy {
  const item = record(value, "automatic grant copy");
  const disclosure = item.disclosure;
  const present = (key: string) =>
    item[key] !== null && item[key] !== undefined;
  let chosen: ScrubCopy;
  if (disclosure === "patterns_only") {
    if (present("model_scrubbed")) {
      throw new Error("Patterns-only grant copy carried model-scrub wording");
    }
    chosen = scrub(item.patterns_only);
  } else if (disclosure === "model_scrubbed") {
    if (present("patterns_only")) {
      throw new Error("Model-scrubbed grant copy carried a second wording");
    }
    chosen = scrub(item.model_scrubbed);
  } else {
    throw new Error("Unknown grant disclosure");
  }
  return {
    disclosure,
    scrub: chosen,
    no_review: text(item, "no_review"),
    scope_required: text(item, "scope_required"),
    path_automatic: text(item, "path_automatic"),
    path_ask_first: text(item, "path_ask_first"),
    raw_send: text(item, "raw_send"),
  };
}

/** The scrub disclosure screen's sentences, in order. */
export function scrubDisclosureLines(copy: AutomaticGrantCopy): string[] {
  return [copy.scrub.scope, copy.scrub.limit, copy.no_review];
}
