// Kept free of imports so the node test runner can load it directly.

/**
 * The connect-inference step's sentences (K12), from
 * `consent_copy::inference_connection_copy`. An offer's own disclosure comes
 * with the offer, chosen by the core per `disclosure_version`.
 */
export type InferenceConnectionCopy = {
  why: string;
  sign_in: string;
  sign_in_failed: string;
  load_failed: string;
  none_offered: string;
  grants_nothing: string;
  one_device: string;
  other_device: string;
  install: string;
  installed: string;
  previous_removed: string;
  reselect: string;
  select_failed: string;
  install_failed: string;
  disconnect: string;
  disconnect_pending: string;
  unknown_disclosure: string;
};

const KEYS = [
  "why",
  "sign_in",
  "sign_in_failed",
  "load_failed",
  "none_offered",
  "grants_nothing",
  "one_device",
  "other_device",
  "install",
  "installed",
  "previous_removed",
  "reselect",
  "select_failed",
  "install_failed",
  "disconnect",
  "disconnect_pending",
  "unknown_disclosure",
] as const;

/** Every sentence or none: a step shown with a sentence missing is refused. */
export function parseInferenceConnectionCopy(
  value: unknown,
): InferenceConnectionCopy {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid inference connection copy");
  }
  const copy = value as Record<string, unknown>;
  const parsed: Record<string, string> = {};
  for (const key of KEYS) {
    const field = copy[key];
    if (typeof field !== "string" || field.length === 0) {
      throw new Error(`Invalid inference connection copy field: ${key}`);
    }
    parsed[key] = field;
  }
  return parsed as InferenceConnectionCopy;
}
