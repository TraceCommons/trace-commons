// Kept free of imports so the node test runner can load it directly.
//
// The two notices the connect-and-forget design requires before the
// automatic-contribution gate is enforced: a folder armed under the old
// "will be scrubbed" wording, told what its arming now means
// (`status.arming_rewordings`), and armed folders the gate is holding
// (`status.automatic_contribution_held`). The words for both come from the
// contributor core; this module only checks the shapes.

/**
 * The daemon's health label while the gate holds armed folders. The words
 * come from `status.automatic_contribution_held` through the core's notice;
 * this label only tells the generic health line to step aside for it.
 */
export const GATE_HELD_LABEL = "automatic-contribution-held";

/**
 * One element of `status.arming_rewordings`. Only `id` is read; the rest is
 * handed back to the core untouched, which words the notice.
 */
export type ArmingRewording = {
  id: number;
  wire: Record<string, unknown>;
};

/** The notice the contributor core assembled for one rewording. */
export type ArmingRewordedNotice = {
  title: string;
  body: string;
  now_heading: string;
  scope: string;
  limit: string;
  no_review: string;
  acknowledge: string;
  /** "Ask me first", or null when the element names no project. */
  ask_first_action: string | null;
  /** Shown when the switch is refused; null exactly when the button is. */
  ask_first_failed: string | null;
};

/** `status.automatic_contribution_held` while something is held. */
export type GateHeld = {
  held_sessions: number;
  wire: Record<string, unknown>;
};

/** One held folder in the core's notice. */
export type GateHeldProject = {
  project_id: string | null;
  line: string;
  ask_first_action: string | null;
  ask_first_failed: string | null;
};

/** The notice the contributor core assembled for what the gate holds. */
export type GateHeldNotice = {
  title: string;
  body: string;
  reasons: string[];
  release: string;
  ask_first: string;
  projects: GateHeldProject[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid notice field: ${key}`);
  }
  return field;
}

function optionalText(
  value: Record<string, unknown>,
  key: string,
): string | null {
  if (!(key in value)) throw new Error(`Invalid notice field: ${key}`);
  const field = value[key];
  if (field === null) return null;
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid notice field: ${key}`);
  }
  return field;
}

/** A button and its refusal line travel together. */
function actionPair(
  value: Record<string, unknown>,
): { action: string | null; failed: string | null } {
  const action = optionalText(value, "ask_first_action");
  const failed = optionalText(value, "ask_first_failed");
  if ((action === null) !== (failed === null)) {
    throw new Error("Invalid notice field: ask_first_failed");
  }
  return { action, failed };
}

/**
 * `undefined` is a core too old to report rewordings, and reads as none.
 * Anything else that is not a list of objects with an id is refused: a
 * malformed list read as empty would reword a folder in silence.
 */
export function parseArmingRewordings(value: unknown): ArmingRewording[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error("Invalid arming_rewordings");
  return value.map((element) => {
    if (
      !isRecord(element) ||
      typeof element.id !== "number" ||
      !Number.isInteger(element.id) ||
      element.id < 0
    ) {
      throw new Error("Invalid arming_rewordings element");
    }
    return { id: element.id, wire: element };
  });
}

/** Shown whole or not at all. */
export function parseArmingRewordedNotice(value: unknown): ArmingRewordedNotice {
  if (!isRecord(value)) throw new Error("Invalid rewording notice");
  const { action, failed } = actionPair(value);
  return {
    title: text(value, "title"),
    body: text(value, "body"),
    now_heading: text(value, "now_heading"),
    scope: text(value, "scope"),
    limit: text(value, "limit"),
    no_review: text(value, "no_review"),
    acknowledge: text(value, "acknowledge"),
    ask_first_action: action,
    ask_first_failed: failed,
  };
}

/**
 * The project "Ask me first" switches, or null when the core offered no
 * button or the element names no project. The button sends
 * `set_project_mode` with this id and `notify_only`, exactly as Settings
 * does.
 */
export function askFirstTarget(
  wire: Record<string, unknown>,
  notice: { ask_first_action: string | null },
): string | null {
  if (notice.ask_first_action === null) return null;
  const id = wire.project_id;
  return typeof id === "string" && id.length > 0 ? id : null;
}

/**
 * `null` when nothing is held, including a core too old to report it.
 * Anything else that is not the object is refused: read as "nothing held",
 * it would hide armed folders the gate is holding.
 */
export function parseGateHeld(value: unknown): GateHeld | null {
  if (value === undefined) return null;
  if (!isRecord(value)) throw new Error("Invalid automatic_contribution_held");
  const held = value.held_sessions;
  if (typeof held !== "number" || !Number.isInteger(held) || held < 0) {
    throw new Error("Invalid automatic_contribution_held field: held_sessions");
  }
  if (held === 0) return null;
  return { held_sessions: held, wire: value };
}

/** Shown whole or not at all. */
export function parseGateHeldNotice(value: unknown): GateHeldNotice {
  if (!isRecord(value)) throw new Error("Invalid held notice");
  const reasons = value.reasons;
  if (
    !Array.isArray(reasons) ||
    reasons.length === 0 ||
    !reasons.every((r) => typeof r === "string" && r.length > 0)
  ) {
    throw new Error("Invalid held notice field: reasons");
  }
  const projects = value.projects;
  if (!Array.isArray(projects)) {
    throw new Error("Invalid held notice field: projects");
  }
  return {
    title: text(value, "title"),
    body: text(value, "body"),
    reasons: reasons as string[],
    release: text(value, "release"),
    ask_first: text(value, "ask_first"),
    projects: projects.map((project) => {
      if (!isRecord(project)) throw new Error("Invalid held notice project");
      const id = project.project_id;
      if (id !== null && (typeof id !== "string" || id.length === 0)) {
        throw new Error("Invalid held notice project: project_id");
      }
      const { action, failed } = actionPair(project);
      return {
        project_id: id,
        line: text(project, "line"),
        ask_first_action: action,
        ask_first_failed: failed,
      };
    }),
  };
}
