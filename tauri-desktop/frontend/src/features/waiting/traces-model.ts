import type { ToolLogoId } from "../../design-system";
import type { HistoryRecord } from "../history/types";
import type { Project, ProjectMode } from "../settings/api/projects-api";
import type { SourceName } from "../settings/api/source-roots-api";
import type { WaitingEntry } from "./types";

/**
 * The Traces tree: tool › folder › session, built only from what the core
 * reports (the waiting queue, the project list, the history record and the
 * source declarations). Nothing here is estimated.
 */

export type ToolSource = {
  name: SourceName;
  /** Source ids the core writes on entries and history records. */
  ids: string[];
  label: string;
  logo: ToolLogoId | null;
};

export const TOOL_SOURCES: ToolSource[] = [
  {
    name: "claude",
    ids: ["claude-code", "claude"],
    label: "Claude Code",
    logo: "claude",
  },
  { name: "codex", ids: ["codex"], label: "Codex", logo: "codex" },
  {
    name: "gemini",
    ids: ["gemini-cli", "gemini"],
    label: "Gemini CLI",
    logo: "anti",
  },
  { name: "cline", ids: ["cline"], label: "Cline", logo: null },
  { name: "opencode", ids: ["opencode"], label: "OpenCode", logo: "opencode" },
];

export const OTHER_TOOL_ID = "other";

export type SourceModeState = "watch" | "off" | "unset";

export type FolderNode = {
  id: string;
  label: string;
  path: string | null;
  mode: ProjectMode | null;
  toolId: string;
  entries: WaitingEntry[];
  contributed: number;
};

export type ToolNode = {
  id: string;
  source: ToolSource | null;
  label: string;
  logo: ToolLogoId | null;
  mode: SourceModeState;
  folders: FolderNode[];
  waiting: number;
  contributed: number;
};

export function toolIdForSource(source: string | null | undefined): string {
  if (!source) return OTHER_TOOL_ID;
  const match = TOOL_SOURCES.find((tool) => tool.ids.includes(source));
  return match ? match.name : OTHER_TOOL_ID;
}

export function sourceMode(
  snapshot: Record<string, unknown> | null,
  name: SourceName,
): SourceModeState {
  const value = snapshot?.[`${name}_source_mode`];
  return value === "watch" ? "watch" : value === "off" ? "off" : "unset";
}

function majority(values: string[]): string | null {
  const counts = new Map<string, number>();
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1);
  let best: string | null = null;
  let bestCount = 0;
  for (const [value, count] of counts) {
    if (count > bestCount) {
      best = value;
      bestCount = count;
    }
  }
  return best;
}

type FolderSeed = {
  id: string;
  label: string;
  path: string | null;
  mode: ProjectMode | null;
  entries: WaitingEntry[];
  history: HistoryRecord[];
};

function seedFolders(
  entries: WaitingEntry[],
  projects: Project[],
  history: HistoryRecord[],
): Map<string, FolderSeed> {
  const seeds = new Map<string, FolderSeed>();
  const seed = (id: string, label: string, path: string | null) => {
    const existing = seeds.get(id);
    if (existing) return existing;
    const created: FolderSeed = {
      id,
      label,
      path,
      mode: null,
      entries: [],
      history: [],
    };
    seeds.set(id, created);
    return created;
  };
  for (const project of projects) {
    if (project.is_unresolved_bucket) continue;
    const folder = seed(
      project.project_id,
      project.project_label,
      project.project_path,
    );
    folder.mode = project.mode;
  }
  for (const entry of entries) {
    seed(entry.project_id, entry.project_label, entry.project_path).entries.push(
      entry,
    );
  }
  for (const record of history) {
    seed(record.project_id, record.project_label, null).history.push(record);
  }
  return seeds;
}

const CONTRIBUTED_STATUSES = new Set(["accepted", "submitted", "quarantined"]);

/**
 * Records that left this machine and still stand: in the commons, waiting
 * to be scored, or held for privacy review. Withdrawn, revoked, purged and
 * expired records are not counted.
 */
export function isContributed(record: HistoryRecord): boolean {
  return CONTRIBUTED_STATUSES.has(record.status);
}

export function buildTraceTree({
  entries,
  projects,
  history,
  snapshot,
  showIgnored,
}: {
  entries: WaitingEntry[];
  projects: Project[];
  history: HistoryRecord[];
  snapshot: Record<string, unknown> | null;
  showIgnored: boolean;
}): ToolNode[] {
  const folders = new Map<string, FolderNode[]>();
  for (const seed of seedFolders(entries, projects, history).values()) {
    if (seed.mode === "ignore" && !showIgnored) continue;
    const toolId = toolIdForSource(
      majority(
        seed.entries
          .map((entry) => entry.source)
          .concat(seed.history.map((record) => record.source)),
      ),
    );
    const list = folders.get(toolId) ?? [];
    list.push({
      id: seed.id,
      label: seed.label,
      path: seed.path,
      mode: seed.mode,
      toolId,
      entries: seed.entries,
      contributed: seed.history.filter(isContributed).length,
    });
    folders.set(toolId, list);
  }
  const activity = (folder: FolderNode) =>
    folder.entries.length + folder.contributed;
  const tools: ToolNode[] = TOOL_SOURCES.map((source) => ({
    id: source.name,
    source,
    label: source.label,
    logo: source.logo,
    mode: sourceMode(snapshot, source.name),
    folders: folders.get(source.name) ?? [],
    waiting: 0,
    contributed: 0,
  }));
  const other = folders.get(OTHER_TOOL_ID);
  if (other?.length) {
    tools.push({
      id: OTHER_TOOL_ID,
      source: null,
      label: "Other folders",
      logo: null,
      mode: "unset",
      folders: other,
      waiting: 0,
      contributed: 0,
    });
  }
  for (const tool of tools) {
    tool.folders.sort((a, b) => activity(b) - activity(a));
    tool.waiting = tool.folders.reduce((n, f) => n + f.entries.length, 0);
    tool.contributed = tool.folders.reduce((n, f) => n + f.contributed, 0);
  }
  return tools.sort(
    (a, b) => b.waiting + b.contributed - (a.waiting + a.contributed),
  );
}

export const MODE_LABEL: Record<ProjectMode, string> = {
  notify_only: "Ask me first",
  auto_upload: "Contribute automatically",
  ignore: "Never offer this one",
};

export const MODE_TONE: Record<ProjectMode, "ask" | "on" | "off"> = {
  notify_only: "ask",
  auto_upload: "on",
  ignore: "off",
};

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return bytes === 0 ? "0 bytes" : `${bytes} bytes`;
}

export function plural(count: number, one: string, many = `${one}s`) {
  return `${count} ${count === 1 ? one : many}`;
}

/* ── Graph ────────────────────────────────────────────────────────── */

export type GraphRange = 0 | 1 | 2;

export type GraphBucket = {
  start: Date;
  end: Date;
  label: string;
  shared: number;
  kept: number;
};

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

function windowFor(range: GraphRange) {
  // 24 hours in 2-hour buckets; 11 days; 42 days.
  if (range === 0) return { count: 12, stepMs: 2 * 3600_000 };
  if (range === 1) return { count: 11, stepMs: 86_400_000 };
  return { count: 42, stepMs: 86_400_000 };
}

function bucketLabel(range: GraphRange, start: Date, index: number) {
  if (range === 0) return index % 3 === 0 ? `${start.getHours()}:00` : "";
  if (range === 1) return DAYS[start.getDay()];
  return index % 6 === 0 ? DAYS[start.getDay()] : "";
}

/**
 * Buckets for the shared-over-kept graph. Shared counts history records by
 * submission time; kept counts sessions still on this machine by discovery
 * time. `offset` steps whole windows back (negative) from now.
 */
export function buildGraphBuckets({
  range,
  offset,
  now,
  sharedTimes,
  keptTimes,
}: {
  range: GraphRange;
  offset: number;
  now: Date;
  sharedTimes: number[];
  keptTimes: number[];
}): GraphBucket[] {
  const { count, stepMs } = windowFor(range);
  const anchor = new Date(now);
  if (range === 0) anchor.setMinutes(0, 0, 0);
  else anchor.setHours(0, 0, 0, 0);
  const endMs = anchor.getTime() + stepMs + offset * count * stepMs;
  const buckets: GraphBucket[] = [];
  for (let index = 0; index < count; index++) {
    const start = new Date(endMs - (count - index) * stepMs);
    const end = new Date(start.getTime() + stepMs);
    const inBucket = (time: number) =>
      time >= start.getTime() && time < end.getTime();
    buckets.push({
      start,
      end,
      label: bucketLabel(range, start, index),
      shared: sharedTimes.filter(inBucket).length,
      kept: keptTimes.filter(inBucket).length,
    });
  }
  return buckets;
}

export function formatDay(date: Date): string {
  return `${["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][date.getDay()]}, ${MONTHS[date.getMonth()]} ${date.getDate()}`;
}

export function rangeLabel(range: GraphRange, offset: number): string {
  if (offset === 0)
    return ["Last 24 hours", "Last 11 days", "Last 42 days"][range];
  const span = ["24 hours", "11 days", "42 days"][range];
  const back = -offset;
  return `${span}, ${back} ${back === 1 ? "window" : "windows"} back`;
}
