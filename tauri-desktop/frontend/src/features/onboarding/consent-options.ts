import type { ConsentOption } from "./types";

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null)
    throw new Error(`Invalid onboarding ${label}`);
  return value as Record<string, unknown>;
}
function string(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid onboarding field: ${key}`);
  return value[key] as string;
}
// A scope's title is the core's short label for it (`consent_options`). No
// shell keeps its own table of them (owner ruling, 2026-10-06): a list
// carrying a scope without one is refused whole, as macOS and GTK refuse it,
// so no row is drawn under words this shell made up. Granted scopes come from
// the daemon's status, not from this list, so a refusal changes no grant.
function title(value: Record<string, unknown>) {
  const raw = value.title;
  if (typeof raw !== "string" || raw.trim() === "")
    throw new Error("Invalid onboarding field: title");
  return raw;
}
// A scope's tag is the core's (`consent_copy::scope_tag`). A daemon from
// before the tag sends none, and the row is then drawn without one rather
// than refused: the tag adds to the title, and the shell types no stand-in.
function tag(value: Record<string, unknown>): string | null {
  const raw = value.tag;
  if (raw === undefined || raw === null || raw === "") return null;
  if (typeof raw !== "string") throw new Error("Invalid onboarding field: tag");
  return raw;
}
function boolean(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "boolean")
    throw new Error(`Invalid onboarding field: ${key}`);
  return value[key] as boolean;
}

/** The core's `consent_options` answer (`daemon::enroll::consent_options`). */
export function parseConsentOptions(value: unknown): ConsentOption[] {
  const response = record(value, "consent response");
  if (!Array.isArray(response.scopes))
    throw new Error("Invalid onboarding scopes");
  return response.scopes.map((entry) => {
    const scope = record(entry, "scope");
    return {
      name: string(scope, "name"),
      title: title(scope),
      description: string(scope, "description"),
      always_on: boolean(scope, "always_on"),
      grants_data_use: boolean(scope, "grants_data_use"),
      tag: tag(scope),
    };
  });
}
