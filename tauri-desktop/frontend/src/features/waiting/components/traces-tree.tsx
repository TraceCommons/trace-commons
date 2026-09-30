import { useState } from "react";
import { ListRow } from "../../../design-system";
import { useEligibilityGroupCopy } from "../../../lib/tauri/use-contributor-copy";
import { useDirectoryPicker } from "../../../lib/tauri/use-platform-actions";
import { useSourceRoots } from "../../settings/public";
import {
  type FolderNode,
  formatBytes,
  MODE_LABEL,
  plural,
  type ToolNode,
} from "../traces-model";
import { useTracesWorkspace } from "../traces-workspace";
import type { WaitingEntry } from "../types";
import { IgnoreProjectControl } from "./ignore-project-control";

type RowMenu = { id: string; kind: "folder" | "session"; label: string };

/**
 * Traces tree: tool › folder › session. The tool switch is the source
 * declaration (watch / off); the folder switch is whether the folder is
 * offered at all (Never offer is off); a folder's Submit pill submits its
 * eligible sessions, the same action as "Submit all eligible"; a session's
 * pill opens its review, where the consent sentence sits above Contribute.
 */
export function TracesTree() {
  const workspace = useTracesWorkspace();
  const [menu, setMenu] = useState<RowMenu | null>(null);
  const [ignoring, setIgnoring] = useState<FolderNode | null>(null);
  const { waiting, tree } = workspace;

  if (waiting.state === "loading")
    return <p className="m-3 tc-body tc-text-tertiary">Reading local queue…</p>;
  if (waiting.state === "error")
    return (
      <div className="tc-alert mx-2">
        The watcher isn't running. It didn't answer. Nothing is being noticed
        or sent while it's stopped, and sessions already waiting stay on this
        machine.
      </div>
    );

  return (
    <div role="tree" aria-label="Tools, folders and sessions" className="relative flex flex-col gap-0.5">
      {tree.map((tool) => (
        <ToolBranch
          key={tool.id}
          tool={tool}
          menu={menu}
          onMenu={setMenu}
          onIgnoreFolder={setIgnoring}
        />
      ))}
      {ignoring ? (
        <IgnoreProjectControl
          projectId={ignoring.id}
          label={ignoring.label}
          pending={ignoring.entries.length}
          disabled={false}
          hideTrigger
          open
          onOpenChange={(open) => {
            if (!open) setIgnoring(null);
          }}
          onIgnore={async (id) => {
            await workspace.projects.setMode(id, "ignore");
            await workspace.waiting.refresh();
          }}
        />
      ) : null}
    </div>
  );
}

function ToolBranch({
  tool,
  menu,
  onMenu,
  onIgnoreFolder,
}: {
  tool: ToolNode;
  menu: RowMenu | null;
  onMenu: (menu: RowMenu | null) => void;
  onIgnoreFolder: (folder: FolderNode) => void;
}) {
  const workspace = useTracesWorkspace();
  const roots = useSourceRoots();
  const picker = useDirectoryPicker();
  const [error, setError] = useState<string | null>(null);
  const open = workspace.expanded.includes(tool.id);
  const off = tool.mode !== "watch";
  const sub =
    tool.mode === "off"
      ? "not watched"
      : tool.mode === "unset" && tool.source
        ? "folder not set"
        : `${plural(tool.waiting, "session")} waiting · ${tool.contributed} contributed`;

  const toggleWatch = async (next: boolean) => {
    if (!tool.source) return;
    setError(null);
    try {
      if (!next) {
        await roots.save(tool.source.name, "off", "");
        return;
      }
      // The core does not return saved paths, so watching needs a folder.
      const path = await picker.pick("source_root");
      await roots.save(tool.source.name, "watch", path);
    } catch {
      setError(
        next
          ? "Folder not chosen, so nothing changed."
          : "The source was not changed.",
      );
    }
  };

  return (
    <>
      <ListRow
        depth={0}
        logo={tool.logo}
        title={tool.label}
        sub={sub}
        off={off && tool.source !== null}
        selected={
          workspace.selection?.kind === "tool" &&
          workspace.selection.id === tool.id
        }
        expanded={open}
        onToggleExpand={
          tool.folders.length ? () => workspace.toggleExpanded(tool.id) : undefined
        }
        onSelect={() =>
          workspace.select(
            workspace.selection?.kind === "tool" &&
              workspace.selection.id === tool.id
              ? null
              : { kind: "tool", id: tool.id },
          )
        }
        watched={tool.source ? tool.mode === "watch" : null}
        watchLabel={`Watch ${tool.label}`}
        watchDisabled={roots.busy || picker.isPending}
        onToggleWatch={(next) => void toggleWatch(next)}
      />
      {error ? (
        <p className="m-0 px-11 tc-caption tc-text-outside" role="alert">
          {error}
        </p>
      ) : null}
      {open
        ? tool.folders.map((folder) => (
            <FolderBranch
              key={folder.id}
              folder={folder}
              toolOff={off && tool.source !== null}
              menu={menu}
              onMenu={onMenu}
              onIgnore={() => onIgnoreFolder(folder)}
            />
          ))
        : null}
    </>
  );
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: a folder row wires its Submit, watch switch and row menu to the queue.
function FolderBranch({
  folder,
  toolOff,
  menu,
  onMenu,
  onIgnore,
}: {
  folder: FolderNode;
  toolOff: boolean;
  menu: RowMenu | null;
  onMenu: (menu: RowMenu | null) => void;
  onIgnore: () => void;
}) {
  const workspace = useTracesWorkspace();
  const eligibility = useEligibilityGroupCopy(folder.entries);
  const open = workspace.expanded.includes(folder.id);
  const ignored = folder.mode === "ignore";
  const eligible =
    eligibility.data?.can_contribute === true
      ? eligibility.data.eligible_count
      : 0;
  const busy =
    workspace.bulk.busyId === folder.id || workspace.projects.state === "busy";
  const menuOpen = menu?.id === folder.id;
  const sub = ignored
    ? "ignored · never queued"
    : `${folder.mode ? `${MODE_LABEL[folder.mode]} · ` : ""}${plural(folder.entries.length, "session")} waiting`;

  return (
    <>
      <div className="relative">
        <ListRow
          depth={1}
          title={folder.label}
          sub={sub}
          off={toolOff || ignored}
          selected={
            workspace.selection?.kind === "folder" &&
            workspace.selection.id === folder.id
          }
          expanded={open}
          onToggleExpand={
            folder.entries.length
              ? () => workspace.toggleExpanded(folder.id)
              : undefined
          }
          onSelect={() =>
            workspace.select(
              workspace.selection?.kind === "folder" &&
                workspace.selection.id === folder.id
                ? null
                : { kind: "folder", id: folder.id },
            )
          }
          submitLabel={
            busy ? "Submitting…" : eligible ? `Submit · ${eligible}` : "Submit"
          }
          submitTip="Sends every eligible waiting session here."
          onSubmit={
            eligible && !busy && !ignored
              ? () =>
                  void workspace.bulk.approve(folder.id, folder.label)
              : null
          }
          watched={!ignored}
          watchLabel="Watch this folder"
          watchDisabled={workspace.projects.state === "busy"}
          onToggleWatch={(next) => {
            if (next) void workspace.projects.setMode(folder.id, "notify_only");
            else onIgnore();
          }}
          menuOpen={menuOpen}
          onOpenMenu={
            ignored
              ? null
              : () =>
                  onMenu(
                    menuOpen
                      ? null
                      : {
                          id: folder.id,
                          kind: "folder",
                          label: "Ignore folder / repo",
                        },
                  )
          }
        />
        {menuOpen ? (
          <RowMenuPill
            label={menu.label}
            onSelect={() => {
              onMenu(null);
              onIgnore();
            }}
          />
        ) : null}
      </div>
      {workspace.bulk.messages[folder.id] ? (
        <p className="m-0 px-14 tc-caption tc-text-accent">
          {workspace.bulk.messages[folder.id]}
        </p>
      ) : null}
      {open
        ? folder.entries.map((entry) => (
            <SessionRow
              key={entry.entry_id}
              entry={entry}
              off={toolOff || ignored}
              menu={menu}
              onMenu={onMenu}
            />
          ))
        : null}
    </>
  );
}

function sessionWhen(entry: WaitingEntry) {
  const date = new Date(entry.discovered_at);
  if (Number.isNaN(date.getTime())) return "Session";
  return date.toLocaleString(undefined, {
    weekday: "short",
    hour: "numeric",
    minute: "2-digit",
  });
}

function SessionRow({
  entry,
  off,
  menu,
  onMenu,
}: {
  entry: WaitingEntry;
  off: boolean;
  menu: RowMenu | null;
  onMenu: (menu: RowMenu | null) => void;
}) {
  const workspace = useTracesWorkspace();
  const selected =
    workspace.selection?.kind === "session" &&
    workspace.selection.id === entry.entry_id;
  const tone = entry.attestation_copy?.tone;
  const flagged = tone === "attention" || tone === "refused";
  const menuOpen = menu?.id === entry.entry_id;
  const detail =
    entry.subagents_dropped > 0
      ? "trimmed to fit"
      : entry.attestation_copy?.state_line ?? "waiting";
  return (
    <div className="relative">
      <ListRow
        depth={2}
        title={sessionWhen(entry)}
        sub={`${formatBytes(entry.size_bytes)} · ${detail}`}
        flag={flagged ? "ask" : tone === "clear" ? "on" : null}
        off={off}
        selected={selected}
        onSelect={() =>
          workspace.select(
            selected ? null : { kind: "session", id: entry.entry_id },
          )
        }
        submitLabel={selected ? "Reviewing" : "Review"}
        submitTip="Opens what would be sent. Nothing leaves until you contribute."
        onSubmit={() =>
          workspace.select({ kind: "session", id: entry.entry_id })
        }
        watched={null}
        menuOpen={menuOpen}
        onOpenMenu={() =>
          onMenu(
            menuOpen
              ? null
              : { id: entry.entry_id, kind: "session", label: "Dismiss session" },
          )
        }
      />
      {menuOpen ? (
        <RowMenuPill
          label={menu.label}
          onSelect={() => {
            onMenu(null);
            void workspace.review.dismissMutation.mutateAsync(entry.entry_id);
            if (selected) workspace.select(null);
          }}
        />
      ) : null}
    </div>
  );
}

/** The row menu: one glass pill under the row. */
function RowMenuPill({
  label,
  onSelect,
}: {
  label: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className="tc-btn tc-btn--glass absolute top-[38px] right-1.5 z-30"
      style={{
        background: "rgba(40,42,50,0.92)",
        backdropFilter: "blur(20px) saturate(160%)",
        boxShadow:
          "inset 0 1px 0 rgba(255,255,255,0.25), 0 10px 24px rgba(0,0,0,0.45)",
      }}
      onClick={onSelect}
    >
      <svg
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-hidden="true"
      >
        <circle cx="12" cy="12" r="9" />
        <path d="M5.6 5.6l12.8 12.8" />
      </svg>
      {label}
    </button>
  );
}
