// MOCK DATA. Every value here stands in for a daemon or near.ai response the
// first-run flow does not have yet. Replace the functions in `ftux-api.ts`,
// not the screens, when the backend lands.
import type { RepoRule } from "../ftux-model";
import type { DetectedTool, PastSession, RepoCandidate } from "../types";

export const MOCK_INVITE_PLACEHOLDER =
  "https://issuer.tracecommons.ai/onboard#XSM77847";

export const MOCK_TOOLS: DetectedTool[] = [
  {
    id: "claude",
    badge: "CC",
    name: "Claude Code",
    folder: "~/.claude/projects",
    presence: "found",
    sessionCount: 62,
    detail: "62 sessions · last one 4 hours ago",
  },
  {
    id: "codex",
    badge: "Cx",
    name: "Codex",
    folder: "~/.codex/sessions",
    presence: "missing",
    sessionCount: 0,
    detail: "Not found on this Mac",
    installUrl: "https://github.com/openai/codex",
  },
  {
    id: "antigravity",
    badge: "Ag",
    name: "Antigravity",
    folder: "~/.gemini/tmp",
    presence: "found",
    sessionCount: 6,
    detail: "6 sessions · answers at Google",
  },
];

const FILLER_TITLES = [
  "rate limiter",
  "flaky e2e suite",
  "schema docs",
  "dependency bump",
  "logging cleanup",
  "retry backoff",
];

type SeedSession = [date: string, title: string, minutes: number | null];

// The first sessions per folder are the ones drawn in the design; the rest
// are filler so the counts and "Show all" behave like a real history.
function sessions(
  folder: string,
  count: number,
  seeds: SeedSession[],
): PastSession[] {
  const last = Date.parse(`${seeds[seeds.length - 1][0]}T00:00:00Z`);
  return Array.from({ length: count }, (_, i) => {
    const seed = seeds[i];
    if (seed) {
      return {
        id: `${folder}#${i}`,
        date: `${seed[0]}T00:00:00.000Z`,
        title: seed[1],
        durationMinutes: seed[2],
      };
    }
    const offset = i - seeds.length + 1;
    return {
      id: `${folder}#${i}`,
      date: new Date(last - offset * 2 * 86_400_000).toISOString(),
      title: FILLER_TITLES[i % FILLER_TITLES.length],
      durationMinutes: 20 + ((i * 17) % 95),
    };
  });
}

function repo(
  folder: string,
  rule: RepoRule,
  count: number,
  seeds: SeedSession[],
  note?: string,
): RepoCandidate {
  return {
    folder,
    note,
    defaultRule: rule,
    sessions: sessions(folder, count, seeds),
  };
}

export const MOCK_REPOS: RepoCandidate[] = [
  repo("~/code/orchard-api", "ask", 22, [
    ["2026-09-12", "fix payments retry test", 78],
    ["2026-09-10", "staging credentials rotation", null],
    ["2026-09-09", "migrate config loader", 52],
    ["2026-09-08", "webhook signatures", 41],
  ]),
  repo("~/code/acme-billing", "ask", 9, [
    ["2026-09-15", "invoice PDF export", 51],
    ["2026-09-15", "nightly reconciliation", 124],
  ]),
  repo("~/code/portfolio", "auto", 12, [
    ["2026-09-16", "contact form", 42],
    ["2026-09-15", "hero animation", 30],
  ]),
  repo(
    "~/clients/dashboard",
    "never",
    6,
    [["2026-09-14", "chart filters", 35]],
    "client",
  ),
];

// Which past sessions start ticked, per folder, matching the design mock.
export const MOCK_DEFAULT_SELECTION: Record<string, number[]> = {
  "~/code/orchard-api": [0, 2, 3],
  "~/code/acme-billing": [0, 1],
  "~/code/portfolio": [0, 1],
};

export const MOCK_ISSUER = {
  host: "issuer.tracecommons.ai",
  payRange: "Pays 0.5 to 2 credits per accepted trace",
};

export const MOCK_STORED_PASSKEY = {
  name: "My trace passkey",
  store: "passwords" as const,
};
