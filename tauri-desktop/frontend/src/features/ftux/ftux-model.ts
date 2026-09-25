// Pure state for the first-run flows in the WYSIWYG design: "Connect and
// forget" (Join -> Folders -> Uses), "Customize and tailor" (Join -> Tools ->
// Rules -> Uses), and the passkey popups that open from the Join screen.
// Kept free of React and of imports so `node --test` can load it directly.

export type FtuxPath = "connect" | "customize";

export type FtuxScreen = "join" | "folders" | "tools" | "rules" | "uses";

export type FtuxStep = { screen: FtuxScreen; label: string };

const CONNECT_STEPS: FtuxStep[] = [
  { screen: "join", label: "Join" },
  { screen: "folders", label: "Folders" },
  { screen: "uses", label: "Uses" },
];

const CUSTOMIZE_STEPS: FtuxStep[] = [
  { screen: "join", label: "Join" },
  { screen: "tools", label: "Tools" },
  { screen: "rules", label: "Rules" },
  { screen: "uses", label: "Uses" },
];

export function stepsFor(path: FtuxPath): FtuxStep[] {
  return path === "connect" ? CONNECT_STEPS : CUSTOMIZE_STEPS;
}

export function stepIndex(path: FtuxPath, screen: FtuxScreen): number {
  return stepsFor(path).findIndex((step) => step.screen === screen);
}

export function nextScreen(
  path: FtuxPath,
  screen: FtuxScreen,
): FtuxScreen | null {
  const steps = stepsFor(path);
  const index = stepIndex(path, screen);
  return index >= 0 && index < steps.length - 1
    ? steps[index + 1].screen
    : null;
}

// Switching tiers keeps the person on the equivalent screen: the folder
// question and the tools screen ask the same thing.
export function switchPath(
  to: FtuxPath,
  screen: FtuxScreen,
): { path: FtuxPath; screen: FtuxScreen } {
  if (to === "customize" && screen === "folders") {
    return { path: to, screen: "tools" };
  }
  if (to === "connect" && (screen === "tools" || screen === "rules")) {
    return { path: to, screen: "folders" };
  }
  return { path: to, screen };
}

// Folders / tools ---------------------------------------------------------

export type WatchAnswer = "unanswered" | "watch" | "ignore";

export type ToolPresence = "found" | "missing";

export type WatchTool = { id: string; presence: ToolPresence };

// "Saying no is an answer": Continue stays off until every tool found on
// this machine has an answer. A missing tool asks again once installed.
export function unansweredTools(
  tools: WatchTool[],
  answers: Record<string, WatchAnswer | undefined>,
): string[] {
  return tools
    .filter((tool) => tool.presence === "found")
    .filter((tool) => (answers[tool.id] ?? "unanswered") === "unanswered")
    .map((tool) => tool.id);
}

export function canContinueFromFolders(
  tools: WatchTool[],
  answers: Record<string, WatchAnswer | undefined>,
): boolean {
  return unansweredTools(tools, answers).length === 0;
}

// Uses --------------------------------------------------------------------

export const OPTIONAL_USES = [
  "Turn my traces into test cases",
  "Train models that judge agent output",
  "Train coding models directly",
] as const;

export function optionalUsesLabel(selected: boolean[]): string {
  const on = selected.filter(Boolean).length;
  const base = `${selected.length} optional uses`;
  if (on === selected.length) return `${base}, all on`;
  if (on === 0) return `${base}, all off`;
  return `${base} · ${on} on`;
}

export type GroupState = "all" | "some" | "none";

export function groupState(selected: boolean[]): GroupState {
  const on = selected.filter(Boolean).length;
  if (selected.length > 0 && on === selected.length) return "all";
  return on === 0 ? "none" : "some";
}

// The group checkbox turns everything on unless everything is already on.
export function toggleGroup(selected: boolean[]): boolean[] {
  const next = groupState(selected) !== "all";
  return selected.map(() => next);
}

export function toggleAt(selected: boolean[], index: number): boolean[] {
  return selected.map((value, i) => (i === index ? !value : value));
}

// Rules and past sessions -------------------------------------------------

export type RepoRule = "ask" | "auto" | "never";

export const REPO_RULE_LABELS: Record<RepoRule, string> = {
  ask: "Ask me",
  auto: "Share automatically",
  never: "Never",
};

export type RepoSelection = {
  folder: string;
  rule: RepoRule;
  sessionCount: number;
  selected: boolean[];
};

export function pastSessionSummary(repos: RepoSelection[]): {
  selected: number;
  eligible: number;
  label: string;
} {
  const eligibleRepos = repos.filter((repo) => repo.rule !== "never");
  const selected = eligibleRepos.reduce(
    (sum, repo) => sum + repo.selected.filter(Boolean).length,
    0,
  );
  const eligible = eligibleRepos.reduce(
    (sum, repo) => sum + repo.sessionCount,
    0,
  );
  return { selected, eligible, label: `${selected} of ${eligible} selected` };
}

// A folder set to Never contributes nothing, including its past sessions.
export function applyRule(repo: RepoSelection, rule: RepoRule): RepoSelection {
  return {
    ...repo,
    rule,
    selected: rule === "never" ? repo.selected.map(() => false) : repo.selected,
  };
}

// Passkey popups ----------------------------------------------------------

// P-1 choose, P-2 name, P-3 system "Save a passkey?", P-4 system Touch ID,
// P-5 verify (binds to near.ai), P-6 system "Sign In" for an existing one.
export type PasskeyStep =
  | "choose"
  | "name"
  | "save-where"
  | "touch-id"
  | "verify"
  | "sign-in";

export type PasskeyOutcome = "created" | "signed-in";

export type PasskeyEvent =
  | { type: "use-existing" }
  | { type: "create-new" }
  | { type: "named" }
  | { type: "store-chosen" }
  | { type: "touched" }
  | { type: "verified" }
  | { type: "back" }
  | { type: "cancel" };

export type PasskeyTransition =
  | { kind: "step"; step: PasskeyStep }
  | { kind: "done"; outcome: PasskeyOutcome }
  // Closing the stack before a passkey exists returns to Join unchanged.
  | { kind: "closed" }
  // Cancelling Verify signs out rather than leaving a half-linked account.
  | { kind: "signed-out" };

// For each step: where each event leads, and where anything else (Cancel,
// Close, Escape) leads.
const PASSKEY_TABLE: Record<
  PasskeyStep,
  {
    on: Partial<Record<PasskeyEvent["type"], PasskeyTransition>>;
    otherwise: PasskeyTransition;
  }
> = {
  choose: {
    on: {
      "use-existing": { kind: "step", step: "sign-in" },
      "create-new": { kind: "step", step: "name" },
    },
    otherwise: { kind: "closed" },
  },
  name: {
    on: {
      named: { kind: "step", step: "save-where" },
      back: { kind: "step", step: "choose" },
    },
    otherwise: { kind: "closed" },
  },
  "save-where": {
    on: { "store-chosen": { kind: "step", step: "touch-id" } },
    otherwise: { kind: "step", step: "name" },
  },
  "touch-id": {
    on: { touched: { kind: "step", step: "verify" } },
    otherwise: { kind: "step", step: "save-where" },
  },
  verify: {
    on: { verified: { kind: "done", outcome: "created" } },
    otherwise: { kind: "signed-out" },
  },
  "sign-in": {
    on: { touched: { kind: "done", outcome: "signed-in" } },
    otherwise: { kind: "step", step: "choose" },
  },
};

export function passkeyTransition(
  step: PasskeyStep,
  event: PasskeyEvent,
): PasskeyTransition {
  const entry = PASSKEY_TABLE[step];
  return entry.on[event.type] ?? entry.otherwise;
}

export const PASSKEY_NAME_MAX = 64;

export function passkeyNameError(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return "Give the passkey a name you will recognise.";
  if (trimmed.length > PASSKEY_NAME_MAX) {
    return `Keep the name under ${PASSKEY_NAME_MAX} characters.`;
  }
  return null;
}

// Formatting --------------------------------------------------------------

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

// "Sat 12 Sep", read in UTC so a session never shifts a day with the zone.
export function formatSessionDate(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return `${WEEKDAYS[date.getUTCDay()]} ${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]}`;
}

// "52 min", "1 h 18 min", "2 h 04 min".
export function formatDuration(minutes: number): string {
  const total = Math.max(0, Math.round(minutes));
  if (total < 60) return `${total} min`;
  const hours = Math.floor(total / 60);
  const rest = total % 60;
  return `${hours} h ${hours >= 2 ? String(rest).padStart(2, "0") : rest} min`;
}
