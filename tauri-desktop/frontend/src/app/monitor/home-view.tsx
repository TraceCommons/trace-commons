import { EyebrowCard, StatusDot, Tag, TertiaryLink } from "../../design-system";
import { StatCard } from "../../components/stat-card";
import { historyStatusLabel, useHistoryData } from "../../features/history";
import { useMissionDrafts } from "../../features/mission-drafts";
import { plural, useTracesWorkspace } from "../../features/waiting/public";
import type { CoreStatus } from "../../lib/tauri/types";

const TAG_TONE: Record<string, "on" | "ask" | undefined> = {
  accepted: "on",
  submitted: "ask",
  quarantined: "ask",
};

/**
 * Home: watching status, the three counts, and the way into Missions and
 * History. Counts come from the core status, the queue and the history
 * rollup; credit is shown only as recorded (pending until settlement).
 */
export function HomeView({
  status,
  onOpenTraces,
  onOpenMissions,
  onOpenHistory,
}: {
  status: CoreStatus | null;
  onOpenTraces: () => void;
  onOpenMissions: () => void;
  onOpenHistory: () => void;
}) {
  const workspace = useTracesWorkspace();
  const history = useHistoryData();
  const drafts = useMissionDrafts();
  const paused = status?.daemon.paused === true;
  const watching = workspace.tree.filter((tool) => tool.mode === "watch").length;
  const waiting = status?.daemon.queue_depth ?? workspace.entries.length;
  const attention = workspace.entries.filter(
    (entry) =>
      entry.attestation_copy?.tone === "attention" ||
      entry.attestation_copy?.tone === "refused",
  ).length;
  const rollup = history.data?.rollup ?? null;
  const recent = (history.data?.history ?? [])
    .slice()
    .sort((a, b) => Date.parse(b.submitted_at) - Date.parse(a.submitted_at))
    .slice(0, 2);

  return (
    <div className="tc-page">
      <div className="tc-card flex items-center gap-3">
        <StatusDot tone={paused ? "ask" : "on"} ring />
        <div className="min-w-0 flex-1">
          <div className="font-semibold">
            {paused ? "Paused" : `Watching ${plural(watching, "tool")}`}
          </div>
          <div className="tc-label font-normal tc-text-secondary">
            {waiting > 0
              ? `${plural(waiting, "session")} waiting for you${attention ? ` · ${attention} worth a second look` : ""}`
              : "Nothing waiting for you"}
          </div>
        </div>
        <TertiaryLink onClick={onOpenTraces}>Open Traces ›</TertiaryLink>
      </div>
      <div className="grid grid-cols-3 gap-1.5">
        <StatCard label="Waiting" value={`${waiting}`} />
        <StatCard
          label="Contributed"
          value={rollup ? `${rollup.all_time.accepted}` : "—"}
        />
        <StatCard
          label="Credit pending"
          value={rollup ? rollup.credit_pending.toFixed(1) : "—"}
        />
      </div>
      <EyebrowCard
        eyebrow="Missions"
        onClick={onOpenMissions}
        accessory={
          <span className="inline-flex items-center gap-1.5">
            <Tag tone="ask">Drafts</Tag>
            <span className="tc-text-tertiary" aria-hidden="true">
              ›
            </span>
          </span>
        }
      >
        {drafts.drafts.length === 0 ? (
          <span className="tc-label font-normal tc-text-secondary">
            No mission drafts on this machine.
          </span>
        ) : (
          drafts.drafts.slice(0, 2).map((draft, index) => (
            <div
              key={draft.id}
              className={`flex items-center justify-between gap-2.5 ${index ? "pt-1.5 tc-hairline-top" : ""}`}
            >
              <span className="flex min-w-0 flex-col">
                <span className="truncate font-semibold">{draft.id}</span>
                <span className="tc-label font-normal tc-text-secondary">
                  {plural(draft.source_count, "source")}
                </span>
              </span>
              <Tag>{draft.status}</Tag>
            </div>
          ))
        )}
      </EyebrowCard>
      <EyebrowCard
        eyebrow="History"
        onClick={onOpenHistory}
        accessory={
          <span className="inline-flex items-center gap-1.5">
            {rollup ? (
              <span className="tc-caption tc-text-secondary">
                {rollup.credit_pending.toFixed(1)} credit pending
              </span>
            ) : null}
            <span className="tc-text-tertiary" aria-hidden="true">
              ›
            </span>
          </span>
        }
      >
        {recent.length === 0 ? (
          <span className="tc-label font-normal tc-text-secondary">
            Nothing contributed from this machine yet.
          </span>
        ) : (
          recent.map((record, index) => (
            <div
              key={record.submission_id}
              className={`flex items-center justify-between gap-2.5 ${index ? "pt-1.5 tc-hairline-top" : ""}`}
            >
              <span className="flex min-w-0 flex-col">
                <span className="truncate font-semibold">
                  {record.project_label}
                </span>
                <span className="tc-label font-normal tc-text-secondary">
                  {new Date(record.submitted_at).toLocaleDateString(undefined, {
                    weekday: "short",
                  })}{" "}
                  · {record.source}
                </span>
              </span>
              <Tag tone={TAG_TONE[record.status]}>
                {historyStatusLabel(record.status)}
              </Tag>
            </div>
          ))
        )}
      </EyebrowCard>
    </div>
  );
}
