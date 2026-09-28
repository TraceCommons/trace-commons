// Kept free of imports so the node test runner can load it directly.

/**
 * `status.legacy_invite_migration`: whether moving this legacy invite
 * identity to a NEAR AI account can be offered, and the notice after it
 * moved, until acknowledged. The notice is kept as sent and handed back to
 * the core, which words it; the shell never reads it to choose words.
 */
export type LegacyMigrationStatus = {
  offered: boolean;
  notice: Record<string, unknown> | null;
};

/** The offer's words, from the contributor core. */
export type LegacyMigrationOffer = {
  title: string;
  body: string;
  action: string;
  working: string;
  invite_prompt: string;
  start_failed: string;
};

/** The notice after the move, from the contributor core. */
export type LegacyMigrationNotice = {
  title: string;
  body: string;
  folders: string;
  acknowledge: string;
};

/** What asking the core to move answered. */
export type LegacyMigrationResult =
  | { kind: "migrated" }
  | { kind: "invite_needed"; line: string }
  | { kind: "refused"; label: string; line: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid legacy migration field: ${key}`);
  }
  return field;
}

/**
 * `undefined` is a core too old to know about the move: nothing offered,
 * nothing to show. Anything else malformed is refused, so a notice is never
 * silently dropped.
 */
export function parseLegacyMigrationStatus(
  value: unknown,
): LegacyMigrationStatus {
  if (value === undefined) return { offered: false, notice: null };
  if (!isRecord(value) || typeof value.offered !== "boolean") {
    throw new Error("Invalid legacy_invite_migration");
  }
  const notice = value.notice ?? null;
  if (notice !== null && !isRecord(notice)) {
    throw new Error("Invalid legacy_invite_migration notice");
  }
  return { offered: value.offered, notice };
}

export function parseLegacyMigrationOffer(
  value: unknown,
): LegacyMigrationOffer {
  if (!isRecord(value)) throw new Error("Invalid legacy migration offer");
  return {
    title: text(value, "title"),
    body: text(value, "body"),
    action: text(value, "action"),
    working: text(value, "working"),
    invite_prompt: text(value, "invite_prompt"),
    start_failed: text(value, "start_failed"),
  };
}

export function parseLegacyMigrationNotice(
  value: unknown,
): LegacyMigrationNotice | null {
  if (value === null) return null;
  if (!isRecord(value)) throw new Error("Invalid legacy migration notice");
  return {
    title: text(value, "title"),
    body: text(value, "body"),
    folders: text(value, "folders"),
    acknowledge: text(value, "acknowledge"),
  };
}

export function parseLegacyMigrationResult(
  value: unknown,
): LegacyMigrationResult {
  if (!isRecord(value)) throw new Error("Invalid legacy migration result");
  if (value.migrated === true) return { kind: "migrated" };
  const label = text(value, "refused");
  const line = text(value, "line");
  if (label === "legacy_migration_invite_needed") {
    return { kind: "invite_needed", line };
  }
  return { kind: "refused", label, line };
}
