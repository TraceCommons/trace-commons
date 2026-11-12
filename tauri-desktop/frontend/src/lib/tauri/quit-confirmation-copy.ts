// Kept free of imports so the node test runner can load it directly.

export type QuitConfirmationCopy = {
  title: string;
  body: string;
  confirm: string;
  cancel: string;
};

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid quit confirmation field: ${key}`);
  }
  return field;
}

/**
 * `null` means nothing live would stop, so the app quits without asking.
 * Anything else must be a whole prompt: a missing sentence is refused
 * rather than filled in.
 */
export function parseQuitConfirmationCopy(
  value: unknown,
): QuitConfirmationCopy | null {
  if (value === null) {
    return null;
  }
  if (typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Invalid quit confirmation response");
  }
  const response = value as Record<string, unknown>;
  return {
    title: text(response, "title"),
    body: text(response, "body"),
    confirm: text(response, "confirm"),
    cancel: text(response, "cancel"),
  };
}
