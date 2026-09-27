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
  /**
   * The signing address of the witness the witness screen showed, `null`
   * for none. The daemon refuses the grant when the witness configured by
   * then is a different one (`automatic-grant-witness-changed`).
   */
  witnessShown: string | null;
};

export const initialFlow1Progress: Flow1Progress = {
  connected: false,
  scopesSaved: null,
  path: null,
  scrubDisclosureSeen: false,
  witnessDisclosureSeen: false,
  witnessShown: null,
};

// The order is the spec's (connect-and-forget design, R7): source roots are
// settled before connect, the scope picker runs immediately after connect
// and before the path question, and the automatic path reaches the grant
// only through both disclosure screens.
export type OnboardingStep =
  | "welcome"
  | "roots"
  | "connect"
  | "consent"
  | "path"
  | "privacy"
  | "disclosure_scrub"
  | "disclosure_witness"
  | "grant"
  | "projects"
  | "done";

export function afterPrivacy(path: ContributionPath | null): OnboardingStep {
  return path === "automatic" ? "disclosure_scrub" : "projects";
}

export function previousStep(
  current: OnboardingStep,
  privacyIncluded: boolean,
  path: ContributionPath | null,
  scopesChosen: boolean,
): OnboardingStep {
  switch (current) {
    case "roots":
      return "welcome";
    case "connect":
      return "roots";
    case "consent":
      return "connect";
    case "path":
      return "consent";
    case "privacy":
      return scopesChosen ? "path" : "consent";
    case "disclosure_scrub":
      return privacyIncluded ? "privacy" : "path";
    case "disclosure_witness":
      return "disclosure_scrub";
    case "grant":
      return "disclosure_witness";
    case "projects":
      if (privacyIncluded) return "privacy";
      return scopesChosen && path !== null ? "path" : "consent";
    default:
      return current;
  }
}

/**
 * Back, as a step and the progress it leaves. Going back undoes the
 * disclosures read: both screens, in order, stand between the contributor
 * and the grant again, and the witness recorded is the one they see next.
 */
export function goBack(
  progress: Flow1Progress,
  current: OnboardingStep,
  privacyIncluded: boolean,
): { step: OnboardingStep; progress: Flow1Progress } {
  return {
    step: previousStep(
      current,
      privacyIncluded,
      progress.path,
      progress.scopesSaved !== null,
    ),
    progress: {
      ...progress,
      scrubDisclosureSeen: false,
      witnessDisclosureSeen: false,
      witnessShown: null,
    },
  };
}

/**
 * R7: declining to choose is not a floor-scope grant. Nothing is saved (the
 * progress holds no scope, and the hook makes no daemon call), no grant is
 * possible, and the contributor lands on Flow 2.
 */
export function decideLater(showPrivacy: boolean): {
  step: OnboardingStep;
  progress: Flow1Progress;
  privacyIncluded: boolean;
} {
  return {
    step: showPrivacy ? "privacy" : "projects",
    progress: { ...initialFlow1Progress, path: "ask_first" },
    privacyIncluded: showPrivacy,
  };
}

/** The witness screen read, and the witness it showed (`null` for none). */
export function acknowledgeWitnessDisclosure(
  progress: Flow1Progress,
  signingAddress: string | null,
): Flow1Progress {
  return {
    ...progress,
    witnessDisclosureSeen: true,
    witnessShown: signingAddress,
  };
}

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
  grant: (witnessSigningAddress: string | null) => Promise<T>,
): Promise<T> {
  if (grantBlockers(progress).length > 0) {
    throw new Error("grant-steps-incomplete");
  }
  return grant(progress.witnessShown);
}

export type WithdrawOutcome = "withdrawn" | "still_granted";

/**
 * Withdraw the grant, then confirm it from the daemon's status rather than
 * from the withdraw call's answer: the confirmation a contributor sees says
 * what is in force now. A failed withdraw throws and is never reported as
 * one; the status is not read.
 */
export async function withdrawAndConfirm(
  withdraw: () => Promise<boolean>,
  readGrant: () => Promise<{ granted: boolean }>,
): Promise<WithdrawOutcome> {
  await withdraw();
  const after = await readGrant();
  return after.granted ? "still_granted" : "withdrawn";
}
