import { type ReactNode, useState } from "react";
import {
  Expander,
  KeyValueList,
  LegendCell,
  StatusDot,
  ToolTile,
} from "../../design-system";
import type { CoreStatus } from "../../lib/tauri/types";
import { useContributorDisclosureCopy } from "../../lib/tauri/use-contributor-copy";
import { ProjectModeField } from "../settings/public";
import { ArmingOffer } from "./components/arming-offer";
import { CertificatePanel } from "./components/certificate-panel";
import { PreviewInspector } from "./components/preview-inspector";
import { PrivateInferenceOffer } from "./components/private-inference-offer";
import { QueueOutcomeDisclosure } from "./components/queue-outcome-disclosure";
import { QueueStatusPanel } from "./components/queue-status-panel";
import { UndoBar } from "./components/undo-bar";
import { WaitingProjectFolder } from "./components/waiting-project-folder";
import { WaitingReview } from "./components/waiting-review";
import { useArmingOffer } from "./hooks/use-arming-offer";
import { useCertificateCopy } from "./hooks/use-certificate-copy";
import { usePrivateInferenceOffer } from "./hooks/use-private-inference-offer";
import { useQueueOutcomeCounts } from "./hooks/use-queue-outcome-counts";
import {
  type FolderNode,
  formatBytes,
  isContributed,
  MODE_LABEL,
  plural,
  type ToolNode,
} from "./traces-model";
import { useTracesWorkspace } from "./traces-workspace";

/**
 * The Traces inspector. With nothing selected it is the summary: what is
 * waiting, what went, today's upload budget and the queue's safeguards.
 * With a tool, folder or session selected it shows that item; a session
 * opens its review, where "Exactly what would be sent" and Contribute live.
 */
export function WaitingPage({ status }: { status: CoreStatus | null }) {
  const workspace = useTracesWorkspace();
  const { selected } = workspace;
  return (
    <div className="tc-page">
      {selected?.kind === "session" ? (
        <SessionInspector />
      ) : selected?.kind === "folder" ? (
        <FolderInspector tool={selected.tool} folder={selected.folder} />
      ) : selected?.kind === "tool" ? (
        <ToolInspector tool={selected.tool} />
      ) : (
        <SummaryInspector status={status} />
      )}
    </div>
  );
}

export function InspectorHeader({
  tile,
  title,
  sub,
}: {
  tile?: ReactNode;
  title: string;
  sub: string;
}) {
  return (
    <div className="flex items-center gap-2.5">
      {tile}
      <div className="min-w-0">
        <div className="truncate text-[17px] font-bold leading-5">{title}</div>
        <div className="truncate tc-caption tc-text-tertiary">{sub}</div>
      </div>
    </div>
  );
}

function Section({
  title,
  children,
  collapsible = false,
}: {
  title: string;
  children: ReactNode;
  collapsible?: boolean;
}) {
  const [open, setOpen] = useState(true);
  return (
    <div className="flex flex-col gap-1.5">
      <Expander open={open} onToggle={collapsible ? () => setOpen(!open) : undefined}>
        {title}
      </Expander>
      {open ? <div className="px-1 tc-label font-normal leading-[17px]">{children}</div> : null}
    </div>
  );
}

function Stat({ label, lines }: { label: string; lines: Array<[string, string]> }) {
  return (
    <div>
      <div className="tc-caption tc-text-tertiary">{label}</div>
      {lines.map(([title, sub], index) => (
        <div key={title} className={index ? "mt-1.5" : "mt-0.5"}>
          <div className="text-[14px] font-semibold">{title}</div>
          <div className="tc-caption tc-text-tertiary">{sub}</div>
        </div>
      ))}
    </div>
  );
}

/**
 * What the contributor must be able to see whenever it is live, whatever
 * the inspector is showing: the undo window after an approve, and the
 * core's one-time arming and Private AI offers. The Monitor mounts this at
 * the top of the inspector for every tab, and opens the inspector when one
 * of them appears (`useInspectorDemand`).
 */
export function WaitingPrompts() {
  const { undo } = useTracesWorkspace();
  const arming = useArmingOffer();
  const privateInference = usePrivateInferenceOffer();
  return (
    <>
      <UndoBar
        scope={undo.scope}
        seconds={undo.seconds}
        busy={undo.busy}
        error={undo.error}
        onUndo={() => void undo.undo()}
        onDismiss={undo.dismiss}
      />
      <ArmingOffer
        offer={arming.offer}
        busy={arming.state === "loading" || arming.state === "busy"}
        error={arming.error}
        onAccept={() => void arming.accept()}
        onDecline={() => void arming.decline()}
      />
      <PrivateInferenceOffer
        offered={privateInference.offered}
        busy={privateInference.busy}
        error={privateInference.error}
        onAnswer={(enabled) => void privateInference.answer(enabled)}
      />
    </>
  );
}

/**
 * Something the inspector must show has appeared: an undo window, a review
 * the contributor just opened, a folder submit in flight, or one of the
 * core's offers. Each value is null when there is nothing to show, so a
 * caller can open the inspector on each change to non-null.
 */
export function useInspectorDemand() {
  const workspace = useTracesWorkspace();
  const arming = useArmingOffer();
  const privateInference = usePrivateInferenceOffer();
  return [
    workspace.undo.scope
      ? `undo:${workspace.undo.scope.id}:${workspace.undo.scope.hold_until}`
      : null,
    workspace.selection?.kind === "session"
      ? `review:${workspace.selection.id}`
      : null,
    workspace.bulk.busyId ? `submit:${workspace.bulk.busyId}` : null,
    arming.offer ? `arming:${arming.offer.project_id}` : null,
    privateInference.offered ? "private-ai-offer" : null,
  ];
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: the summary composes the queue safeguards; each block is a presence check.
function SummaryInspector({ status }: { status: CoreStatus | null }) {
  const workspace = useTracesWorkspace();
  const outcomes = useQueueOutcomeCounts();
  const evidenceAdmitted =
    workspace.settings.data?.admission_evidence_required === true;
  const certificateCopy = useCertificateCopy(evidenceAdmitted);
  const { tree, entries } = workspace;
  const rollup = workspace.history.data?.rollup ?? null;
  const records = workspace.history.data?.history ?? [];
  const watched = tree.filter((tool) => tool.mode === "watch").length;
  const sourced = tree.filter((tool) => tool.source).length;
  const folders = tree.reduce((n, tool) => n + tool.folders.length, 0);
  const attention = entries.filter(
    (entry) =>
      entry.attestation_copy?.tone === "attention" ||
      entry.attestation_copy?.tone === "refused",
  ).length;
  const contributed = records.filter(isContributed).length;
  const budget = status?.daemon.daily_budget;
  const topFolders = tree
    .flatMap((tool) => tool.folders)
    .sort(
      (a, b) =>
        b.entries.length + b.contributed - (a.entries.length + a.contributed),
    )
    .slice(0, 2);
  const topTools = tree
    .filter((tool) => tool.waiting + tool.contributed > 0)
    .slice(0, 2);
  return (
    <>
      <InspectorHeader
        title="Summary"
        sub={`${watched} of ${sourced} tools watched · ${plural(folders, "project")} · ${plural(entries.length, "session")} waiting`}
      />
      <div className="tc-legend">
        <LegendCell color="var(--tc-data-shared)" label="shared" value={contributed} />
        <LegendCell color="var(--tc-data-kept)" label="kept" value={entries.length} />
      </div>
      <Section title="Decisions" collapsible>
        <ul className="m-0 flex list-none flex-col gap-2 p-0 text-[14px]">
          <DecisionLine glyph="◫" tone="ask">
            <strong>{status?.daemon.queue_depth ?? entries.length}</strong> waiting for you
          </DecisionLine>
          <DecisionLine glyph="⚠" tone="ask">
            <strong>{attention}</strong> worth a second look
          </DecisionLine>
          <DecisionLine glyph="✓" tone="on">
            <strong>{contributed}</strong> contributed
            {rollup ? ` · ${rollup.credit_pending.toFixed(1)} credit pending` : ""}
          </DecisionLine>
          {budget ? (
            <DecisionLine glyph="⏸">
              <strong>{budget.uploads_today}</strong> of {budget.max_uploads_per_day} uploads today
            </DecisionLine>
          ) : null}
        </ul>
      </Section>
      {topFolders.length || topTools.length ? (
        <Section title="Statistics" collapsible>
          <div className="flex flex-col gap-2.5">
            {topFolders.length ? (
              <Stat
                label="Top projects"
                lines={topFolders.map((folder) => [
                  folder.label,
                  `${folder.entries.length} waiting · ${folder.contributed} contributed${folder.mode ? ` · ${MODE_LABEL[folder.mode]}` : ""}`,
                ])}
              />
            ) : null}
            {topTools.length ? (
              <Stat
                label="Top tools"
                lines={topTools.map((tool) => [
                  tool.label,
                  `${tool.waiting} waiting · ${tool.contributed} contributed`,
                ])}
              />
            ) : null}
          </div>
        </Section>
      ) : null}
      {status && (
        <QueueStatusPanel
          health={status.daemon.health}
          budget={status.daemon.daily_budget}
          routing={status.daemon.routing}
          witnessCapacity={status.daemon.witness_capacity}
          gateHeld={status.daemon.automatic_contribution_held}
        />
      )}
      <CertificatePanel entries={entries} copy={certificateCopy.data ?? null} />
      <QueueOutcomeDisclosure
        reasons={outcomes.data?.reasons ?? null}
        lines={outcomes.data?.lines ?? {}}
      />
    </>
  );
}

function DecisionLine({
  glyph,
  tone,
  children,
}: {
  glyph: string;
  tone?: "ask" | "on";
  children: ReactNode;
}) {
  return (
    <li className="flex items-center gap-2.5">
      <span
        aria-hidden="true"
        className={`w-5 text-center ${tone === "ask" ? "tc-text-ask" : tone === "on" ? "tc-text-on" : "tc-text-tertiary"}`}
      >
        {glyph}
      </span>
      <span>{children}</span>
    </li>
  );
}

function ToolInspector({ tool }: { tool: ToolNode }) {
  const disclosure = useContributorDisclosureCopy();
  const statusLine = tool.source
    ? disclosure.data?.source_check_lines[tool.source.name]?.[tool.mode]
    : null;
  return (
    <>
      <InspectorHeader
        tile={<ToolTile tool={tool.logo} fallback={tool.label.slice(0, 2)} large />}
        title={tool.label}
        sub="Tool"
      />
      <div className="tc-legend">
        <LegendCell color="var(--tc-data-shared)" label="shared" value={tool.contributed} />
        <LegendCell color="var(--tc-data-kept)" label="kept" value={tool.waiting} />
      </div>
      <Section title="Tool">
        <KeyValueList
          items={[
            {
              label: "Watching",
              value: (
                <span className="inline-flex items-center gap-1.5">
                  <StatusDot tone={tool.mode === "watch" ? "on" : "off"} size="md" />
                  {tool.mode === "watch" ? "On" : tool.mode === "off" ? "Off" : "Not set"}
                </span>
              ),
            },
            { label: "Folders", value: tool.folders.length },
            { label: "Waiting", value: tool.waiting },
          ]}
        />
      </Section>
      {statusLine ? <Section title="Traces folder">{statusLine}</Section> : null}
      <Section title="Decisions">
        {plural(tool.waiting, "trace")} waiting · {tool.contributed} contributed
      </Section>
    </>
  );
}

function FolderInspector({ tool, folder }: { tool: ToolNode; folder: FolderNode }) {
  const workspace = useTracesWorkspace();
  const project = workspace.projects.projects.find(
    (row) => row.project_id === folder.id,
  );
  const bytes = folder.entries.reduce((n, entry) => n + entry.size_bytes, 0);
  return (
    <>
      <InspectorHeader
        tile={<ToolTile kind="folder" large />}
        title={folder.label}
        sub={`Project · ${tool.label}`}
      />
      <div className="tc-legend">
        <LegendCell color="var(--tc-data-shared)" label="shared" value={folder.contributed} />
        <LegendCell color="var(--tc-data-kept)" label="kept" value={folder.entries.length} />
      </div>
      <Section title="Project">
        <KeyValueList
          items={[
            { label: "Path", value: folder.path ?? "—", mono: true },
            { label: "Waiting", value: folder.entries.length },
            { label: "Size", value: formatBytes(bytes) },
          ]}
        />
      </Section>
      <Section title="Contribution rule">
        {project ? (
          <ProjectModeField
            project={project}
            allowAutoUpload
            disabled={workspace.projects.state !== "ready"}
            onSetMode={workspace.projects.setMode}
          />
        ) : (
          "This folder has no rule of its own yet."
        )}
      </Section>
      {folder.entries.length ? (
        <Section title="Decisions">
          <WaitingProjectFolder
            projectId={folder.id}
            label={folder.label}
            path={folder.path ?? undefined}
            count={folder.entries.length}
            entries={folder.entries}
            busy={
              workspace.bulk.busyId === folder.id ||
              workspace.projects.state === "busy"
            }
            message={workspace.bulk.messages[folder.id]}
            onOpen={(id) => workspace.reveal([tool.id, id])}
            onSubmitAll={(id) => void workspace.bulk.approve(id, folder.label)}
            onSubmitAllAs={(id, outcome) =>
              void workspace.bulk.approve(id, folder.label, outcome)
            }
            onIgnore={async (id) => {
              await workspace.projects.setMode(id, "ignore");
              await workspace.waiting.refresh();
            }}
          />
        </Section>
      ) : null}
    </>
  );
}

function SessionInspector() {
  const workspace = useTracesWorkspace();
  const { review, waiting, selected } = workspace;
  if (selected?.kind !== "session") return null;
  return (
    <>
      <InspectorHeader
        tile={<ToolTile kind="session" large />}
        title={selected.folder.label}
        sub={`Trace · ${selected.tool.label}`}
      />
      <WaitingReview
        preview={review.preview}
        state={review.state}
        error={review.error}
        errorKind={review.errorKind}
        eligibilityCopy={review.eligibilityCopy}
        eligibilityPending={review.eligibilityPending}
        eligibilityError={review.eligibilityError}
        outcomeCopy={review.outcomeCopy}
        outcomeCopyPending={review.outcomeCopyPending}
        outcomeCopyError={review.outcomeCopyError}
        verdict={review.verdict}
        correction={review.correction}
        credentialRefusal={review.credentialRefusal}
        onVerdictChange={review.setVerdict}
        onCorrectionChange={review.setCorrection}
        onApprove={() => void review.approve()}
        onDismiss={() => {
          void review.dismiss().then((dismissed) => {
            if (dismissed) workspace.select(null);
          });
        }}
        onInspect={() => workspace.setInspecting(true)}
      />
      <PreviewInspector
        preview={review.preview}
        open={workspace.inspecting}
        onClose={() => workspace.setInspecting(false)}
        onReviewed={() => {
          void review.refetch();
          void waiting.refresh();
        }}
      />
    </>
  );
}
