// The Flow 1 order and its guards (connect-and-forget design, R7 and "The
// disclosure"). Kept free of imports so the node test runner can load it.

export type ScopeOption = { name: string; always_on: boolean };

/**
 * R7: the scope picker has no default. Nothing is pre-selected, the floor
 * scope included, so a grant never ships at a scope nobody chose.
 */
export function initialScopeSelection(): string[] {
  return [];
}

export type ScopeChoice = {
  canContinue: boolean;
  missingRequired: string[];
};

/**
 * Whether the picker may continue: something chosen, every always-on scope
 * ticked by hand, and nothing the daemon did not offer.
 */
export function scopeChoice(
  options: ScopeOption[],
  selected: string[],
): ScopeChoice {
  const known = new Set(options.map((option) => option.name));
  const missingRequired = options
    .filter((option) => option.always_on && !selected.includes(option.name))
    .map((option) => option.name);
  const canContinue =
    options.length > 0 &&
    selected.length > 0 &&
    missingRequired.length === 0 &&
    selected.every((name) => known.has(name));
  return { canContinue, missingRequired };
}

export type ContributionPath = "automatic" | "ask_first";

export type Flow1Progress = {
  /** The daemon reports an enrollment. */
  connected: boolean;
  /** Scopes the contributor chose and the daemon saved, this session. */
  scopesSaved: string[] | null;
  path: ContributionPath | null;
  scrubDisclosureSeen: boolean;
  witnessDisclosureSeen: boolean;
};

export const initialFlow1Progress: Flow1Progress = {
  connected: false,
  scopesSaved: null,
  path: null,
  scrubDisclosureSeen: false,
  witnessDisclosureSeen: false,
};

export type GrantBlocker =
  | "connect"
  | "scope"
  | "path"
  | "scrub_disclosure"
  | "witness_disclosure";

/** Every step still standing between the contributor and the grant. */
export function grantBlockers(progress: Flow1Progress): GrantBlocker[] {
  const blockers: GrantBlocker[] = [];
  if (!progress.connected) blockers.push("connect");
  if (!progress.scopesSaved || progress.scopesSaved.length === 0)
    blockers.push("scope");
  if (progress.path !== "automatic") blockers.push("path");
  if (!progress.scrubDisclosureSeen) blockers.push("scrub_disclosure");
  if (!progress.witnessDisclosureSeen) blockers.push("witness_disclosure");
  return blockers;
}

/**
 * The only way the onboarding reaches `grant_automatic`: refused, without
 * calling it, until connect, a chosen scope, the automatic path and both
 * disclosure screens are done.
 */
export async function requestGrant<T>(
  progress: Flow1Progress,
  grant: () => Promise<T>,
): Promise<T> {
  if (grantBlockers(progress).length > 0) {
    throw new Error("grant-steps-incomplete");
  }
  return grant();
}
