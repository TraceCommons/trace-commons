// Kept free of imports so the node test runner can load it directly.
//
// The account section's status while the daemon says this app's account
// needs attention (`status.account_state`). The words, the button and its
// confirmation come from the contributor core; this module only checks the
// shapes and picks the keys the core reads.

/** What the core assembled for the account section. */
export type AccountAttentionCopy = {
  title: string;
  /** The reason's sentence; null for a reason this build does not know. */
  body: string | null;
  /** Present only while a NEAR AI login is stored. */
  private_ai: string | null;
  action: string;
  confirm_title: string;
  confirm_body: string;
  confirm_action: string;
  confirm_cancel: string;
  /** Shown when the daemon refused to set up again. */
  failed: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid account attention field: ${key}`);
  }
  return field;
}

function optionalText(
  value: Record<string, unknown>,
  key: string,
): string | null {
  const field = value[key];
  if (field === null || field === undefined) return null;
  return text(value, key);
}

/**
 * The keys of `status.daemon` the core reads, or null while there is nothing
 * to ask it: the state is `ok`, absent (a daemon too old to say) or unknown.
 */
export function accountAttentionWire(
  daemon: unknown,
): Record<string, unknown> | null {
  if (!isRecord(daemon) || daemon.account_state !== "needs_attention") {
    return null;
  }
  return {
    account_state: daemon.account_state,
    account_state_reason: daemon.account_state_reason ?? null,
    near_ai_login_unconfirmed: daemon.near_ai_login_unconfirmed === true,
  };
}

/** The core's answer: null when it has nothing to show. */
export function parseAccountAttentionCopy(
  value: unknown,
): AccountAttentionCopy | null {
  if (value === null) return null;
  if (!isRecord(value)) throw new Error("Invalid account attention copy");
  return {
    title: text(value, "title"),
    body: optionalText(value, "body"),
    private_ai: optionalText(value, "private_ai"),
    action: text(value, "action"),
    confirm_title: text(value, "confirm_title"),
    confirm_body: text(value, "confirm_body"),
    confirm_action: text(value, "confirm_action"),
    confirm_cancel: text(value, "confirm_cancel"),
    failed: text(value, "failed"),
  };
}

/**
 * What setting up again left behind, as the core's sentence, or null when
 * the NEAR AI sign-in went with the enrollment.
 */
export function parseSetUpAgainAnswer(value: unknown): {
  leftBehind: string | null;
} {
  if (!isRecord(value) || !("left_behind" in value)) {
    throw new Error("Invalid set-up-again answer");
  }
  const line = value.left_behind;
  if (line !== null && (typeof line !== "string" || line.length === 0)) {
    throw new Error("Invalid set-up-again answer");
  }
  return { leftBehind: line };
}
