// Kept free of imports so the node test runner can load it directly.

/** Which process is doing the watching, as the Rust core reports it. */
export type QuitRole = "hosting" | "attached" | "unavailable";

export type QuitConfirmationCopy = {
  role: QuitRole;
  title: string;
  body: string;
  confirm: string;
  cancel: string;
};

const roles: readonly QuitRole[] = ["hosting", "attached", "unavailable"];

function text(value: Record<string, unknown>, key: string): string {
  const field = value[key];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Invalid quit confirmation field: ${key}`);
  }
  return field;
}

/**
 * The quit prompt is only true for the role it was chosen for, so an
 * unknown role or a missing sentence is refused rather than filled in.
 */
export function parseQuitConfirmationCopy(
  value: unknown,
): QuitConfirmationCopy {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid quit confirmation response");
  }
  const response = value as Record<string, unknown>;
  const role = roles.find((candidate) => candidate === response.role);
  if (role === undefined) {
    throw new Error("Invalid quit confirmation role");
  }
  return {
    role,
    title: text(response, "title"),
    body: text(response, "body"),
    confirm: text(response, "confirm"),
    cancel: text(response, "cancel"),
  };
}
