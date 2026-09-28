// Kept free of imports so the node test runner can load it directly.

/**
 * The daemon's health label while approved sessions wait on a busy privacy
 * witness. The words for it come from `status.witness_capacity`, through the
 * core's notice; this label only tells the generic health line to step
 * aside for that notice.
 */
export const WITNESS_SATURATED_LABEL = "witness-saturated";

/**
 * `status.witness_capacity` while sessions are waiting. `wire` is handed
 * back to the core untouched, which words the notice; the shell never
 * counts or pluralises by itself.
 */
export type WitnessCapacity = {
  waiting_sessions: number;
  next_retry_at: string | null;
  wire: Record<string, unknown>;
};

/** The notice the contributor core assembled for the waiting sessions. */
export type WitnessCapacityNotice = {
  title: string;
  body: string;
  /** The label beside the next retry time, rendered here in local time. */
  next_check: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * `null` when nothing is waiting, including a core too old to report the
 * object (`undefined`). Anything else that is not the object is refused:
 * read as "nothing waiting", it would hide held sessions.
 */
export function parseWitnessCapacity(value: unknown): WitnessCapacity | null {
  if (value === undefined) return null;
  if (!isRecord(value)) throw new Error("Invalid witness_capacity");
  const waiting = value.waiting_sessions;
  if (typeof waiting !== "number" || !Number.isInteger(waiting) || waiting < 0) {
    throw new Error("Invalid witness_capacity field: waiting_sessions");
  }
  const next = value.next_retry_at;
  if (next !== null && typeof next !== "string") {
    throw new Error("Invalid witness_capacity field: next_retry_at");
  }
  if (waiting === 0) return null;
  return { waiting_sessions: waiting, next_retry_at: next, wire: value };
}

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid witness capacity notice field: ${key}`);
  }
  return field;
}

/** Shown whole or not at all. */
export function parseWitnessCapacityNotice(
  value: unknown,
): WitnessCapacityNotice {
  if (!isRecord(value)) throw new Error("Invalid witness capacity notice");
  return {
    title: text(value, "title"),
    body: text(value, "body"),
    next_check: text(value, "next_check"),
  };
}

/**
 * "Next try: <local time>", or null when the daemon gave no time or one this
 * shell cannot read -- never a guessed time.
 */
export function nextRetryLine(
  notice: WitnessCapacityNotice,
  capacity: WitnessCapacity,
  format: (date: Date) => string = (date) => date.toLocaleString(),
): string | null {
  if (capacity.next_retry_at === null) return null;
  const date = new Date(capacity.next_retry_at);
  if (Number.isNaN(date.getTime())) return null;
  return `${notice.next_check}: ${format(date)}`;
}

/**
 * For a review a person asked for that met a busy witness: "<label>: <local
 * time>", from the daemon's `view` (`state: "Busy"`, `retry_at`,
 * `retry_label`). `null` for any other outcome, or for a time or label this
 * shell cannot read -- never a guessed time.
 */
export function reviewRetryLine(
  view: unknown,
  format: (date: Date) => string = (date) => date.toLocaleString(),
): string | null {
  if (!isRecord(view) || view.state !== "Busy") return null;
  const label = view.retry_label;
  const at = view.retry_at;
  if (typeof label !== "string" || label.length === 0) return null;
  if (typeof at !== "string") return null;
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return null;
  return `${label}: ${format(date)}`;
}
