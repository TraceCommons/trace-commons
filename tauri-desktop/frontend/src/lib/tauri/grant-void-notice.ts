// Kept free of imports so the node test runner can load it directly.

/**
 * One element of `status.grant_voids`: a grant the core voided because the
 * terms it was given under widened, not yet shown to the contributor.
 *
 * Only `id` is read here. The rest is kept as `wire` and handed back to the
 * core untouched, which turns it into the notice; the shell never reads
 * `kind` or `reasons` to choose words.
 */
export type GrantVoid = {
  id: number;
  wire: Record<string, unknown>;
};

/** The notice the contributor core assembled for one void. */
export type GrantVoidNotice = {
  title: string;
  body: string;
  reasons_heading: string;
  reasons: string[];
  rearm: string;
  acknowledge: string;
  /**
   * The re-arm button ("Turn back on"), or null when the notice has no
   * project to arm: the automatic grant's notice and an unplaced one.
   */
  rearm_action: string | null;
  /** Shown when the daemon refuses the re-arm; null exactly when the button is. */
  rearm_failed: string | null;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * `undefined` is a core too old to report voids, and reads as none. Anything
 * else that is not a list of objects with an id is refused: a malformed list
 * read as empty would be a void nobody is told about.
 */
export function parseGrantVoids(value: unknown): GrantVoid[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error("Invalid grant_voids");
  return value.map((element) => {
    if (
      !isRecord(element) ||
      typeof element.id !== "number" ||
      !Number.isInteger(element.id) ||
      element.id < 0
    ) {
      throw new Error("Invalid grant_voids element");
    }
    return { id: element.id, wire: element };
  });
}

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid void notice field: ${key}`);
  }
  return field;
}

/**
 * The project the notice's "Turn back on" arms, or null when there is no
 * button. The button sends `set_project_mode` with this id and
 * `auto_upload`, exactly as arming a project in Settings does. Read from the
 * wire element only when the core offered the button, so the shell never
 * decides by itself which notices can be re-armed.
 */
export function rearmTarget(
  grantVoid: GrantVoid,
  notice: GrantVoidNotice,
): string | null {
  if (notice.rearm_action === null) return null;
  const id = grantVoid.wire.project_id;
  return typeof id === "string" && id.length > 0 ? id : null;
}

function optionalText(
  value: Record<string, unknown>,
  key: string,
): string | null {
  if (!(key in value)) throw new Error(`Invalid void notice field: ${key}`);
  const field = value[key];
  if (field === null) return null;
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid void notice field: ${key}`);
  }
  return field;
}

/**
 * The notice is shown whole or not at all: a missing sentence is refused, so
 * the contributor is never shown that automatic contributing stopped without
 * why, or without how to turn it back on.
 */
export function parseGrantVoidNotice(value: unknown): GrantVoidNotice {
  if (!isRecord(value)) throw new Error("Invalid void notice");
  const reasons = value.reasons;
  if (
    !Array.isArray(reasons) ||
    reasons.length === 0 ||
    !reasons.every((r) => typeof r === "string" && r.length > 0)
  ) {
    throw new Error("Invalid void notice field: reasons");
  }
  const rearmAction = optionalText(value, "rearm_action");
  const rearmFailed = optionalText(value, "rearm_failed");
  if ((rearmAction === null) !== (rearmFailed === null)) {
    throw new Error("Invalid void notice field: rearm_failed");
  }
  return {
    title: text(value, "title"),
    body: text(value, "body"),
    reasons_heading: text(value, "reasons_heading"),
    reasons: reasons as string[],
    rearm: text(value, "rearm"),
    acknowledge: text(value, "acknowledge"),
    rearm_action: rearmAction,
    rearm_failed: rearmFailed,
  };
}
