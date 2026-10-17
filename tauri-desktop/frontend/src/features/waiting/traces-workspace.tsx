import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useMemo,
  useState,
} from "react";
import { useHistoryData } from "../history/hooks/use-history-data";
import { useProjects, useSettings } from "../settings/public";
import { useWaitingBulkApproval } from "./hooks/use-waiting-bulk-approval";
import { useWaitingData } from "./hooks/use-waiting-data";
import { useWaitingReview } from "./hooks/use-waiting-review";
import { useWaitingUndo } from "./hooks/use-waiting-undo";
import { buildTraceTree, type FolderNode, type ToolNode } from "./traces-model";
import type { WaitingEntry } from "./types";

export type TraceSelection =
  | { kind: "tool"; id: string }
  | { kind: "folder"; id: string }
  | { kind: "session"; id: string }
  | null;

export type ResolvedSelection =
  | { kind: "tool"; tool: ToolNode }
  | { kind: "folder"; tool: ToolNode; folder: FolderNode }
  | {
      kind: "session";
      tool: ToolNode;
      folder: FolderNode;
      entry: WaitingEntry;
    }
  | null;

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: one pass over tool › folder › session to find the selection.
function resolve(tree: ToolNode[], selection: TraceSelection): ResolvedSelection {
  if (!selection) return null;
  for (const tool of tree) {
    if (selection.kind === "tool" && tool.id === selection.id)
      return { kind: "tool", tool };
    for (const folder of tool.folders) {
      if (selection.kind === "folder" && folder.id === selection.id)
        return { kind: "folder", tool, folder };
      if (selection.kind !== "session") continue;
      const entry = folder.entries.find((e) => e.entry_id === selection.id);
      if (entry) return { kind: "session", tool, folder, entry };
    }
  }
  return null;
}

function useTracesWorkspaceState() {
  const waiting = useWaitingData();
  const undo = useWaitingUndo();
  const review = useWaitingReview(undo.prepare);
  const bulk = useWaitingBulkApproval(undo.prepare);
  const projects = useProjects();
  const settings = useSettings();
  const history = useHistoryData();
  const [selection, setSelection] = useState<TraceSelection>(null);
  const [expanded, setExpanded] = useState<string[]>([]);
  const [showIgnored, setShowIgnored] = useState(true);
  const [inspecting, setInspecting] = useState(false);

  const entries = waiting.data?.pending ?? [];
  const records = history.data?.history ?? [];
  const tree = useMemo(
    () =>
      buildTraceTree({
        entries,
        projects: projects.projects,
        history: records,
        snapshot: settings.data,
        showIgnored,
      }),
    [entries, projects.projects, records, settings.data, showIgnored],
  );
  const selected = useMemo(() => resolve(tree, selection), [tree, selection]);

  const select = useCallback(
    (next: TraceSelection) => {
      setSelection(next);
      setInspecting(false);
      if (next?.kind === "session") void review.review(next.id);
      else review.clear();
    },
    [review],
  );
  const toggleExpanded = useCallback((id: string) => {
    setExpanded((current) =>
      current.includes(id)
        ? current.filter((item) => item !== id)
        : current.concat(id),
    );
  }, []);
  const reveal = useCallback((ids: string[]) => {
    setExpanded((current) => Array.from(new Set(current.concat(ids))));
  }, []);

  return {
    waiting,
    undo,
    review,
    bulk,
    projects,
    settings,
    history,
    entries,
    tree,
    selection,
    selected,
    select,
    expanded,
    toggleExpanded,
    reveal,
    showIgnored,
    setShowIgnored,
    inspecting,
    setInspecting,
  };
}

export type TracesWorkspace = ReturnType<typeof useTracesWorkspaceState>;

const TracesWorkspaceContext = createContext<TracesWorkspace | null>(null);

/**
 * One queue, one review, one selection, shared by the tree (left pane), the
 * flow map and the inspector.
 */
export function TracesWorkspaceProvider({ children }: { children: ReactNode }) {
  const workspace = useTracesWorkspaceState();
  return (
    <TracesWorkspaceContext.Provider value={workspace}>
      {children}
    </TracesWorkspaceContext.Provider>
  );
}

// biome-ignore lint/style/useComponentExportOnlyModules: the provider and its hook ship together.
export function useTracesWorkspace(): TracesWorkspace {
  const workspace = useContext(TracesWorkspaceContext);
  if (!workspace)
    throw new Error("useTracesWorkspace needs a TracesWorkspaceProvider");
  return workspace;
}
