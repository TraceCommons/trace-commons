// The daemon starts only once Claude Code and Codex are both answered
// (`daemon::settings::roots_declared`): an unanswered one would be read as
// its conventional location. Optional sources never gate the start.
const REQUIRED_ROOTS = ["claude", "codex"] as const;
export type RequiredRoot = (typeof REQUIRED_ROOTS)[number];

export type RootsReadiness = "undeclared" | "needs_start" | "running";

function answered(mode: unknown) {
  return mode === "watch" || mode === "off";
}

export function missingRequiredRoots(
  snapshot: Record<string, unknown>,
): RequiredRoot[] {
  return REQUIRED_ROOTS.filter(
    (source) => !answered(snapshot[`${source}_source_mode`]),
  );
}

export function rootsReadiness(
  snapshot: Record<string, unknown>,
  startup: string | undefined,
): RootsReadiness {
  if (missingRequiredRoots(snapshot).length > 0) return "undeclared";
  return startup === "running" ? "running" : "needs_start";
}

export function rootsContinueError(
  error: unknown,
): "roots_required" | "start_failed" {
  return error instanceof Error && error.message === "roots-not-declared"
    ? "roots_required"
    : "start_failed";
}
