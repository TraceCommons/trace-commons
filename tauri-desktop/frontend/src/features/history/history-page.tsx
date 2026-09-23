import { useMemo, useState } from "react";
import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import { CommunityPanel } from "./components/community-panel";
import { CreditRecordPanel } from "./components/credit-record-panel";
import { HistoryDetailView } from "./components/history-detail";
import {
  type HistoryFilter,
  HistoryFilterBar,
} from "./components/history-filter";
import { HistoryRow } from "./components/history-row";
import { HistoryRefreshControl } from "./components/history-refresh-control";
import { PublicRunEditor } from "./components/public-run-editor";
import { SkillLearningPanel } from "./components/skill-learning-panel";
import {
  countHistory,
  filterHistory,
  groupHistory,
  quarantineExplanations,
} from "./history-view-model";
import { useHistoryData } from "./hooks/use-history-data";
import { useHistoryDetail } from "./hooks/use-history-detail";
import { useHistoryWithdrawal } from "./hooks/use-history-withdrawal";
import { useContributorDisclosureCopy } from "../../lib/tauri/use-contributor-copy";

export function HistoryPage() {
  const history = useHistoryData();
  const rollup = history.data?.rollup;
  const records = history.data?.history ?? [];
  const detail = useHistoryDetail();
  const withdrawal = useHistoryWithdrawal();
  const contributorCopy = useContributorDisclosureCopy();
  const heldRowFallback = contributorCopy.data?.history_ui.held_row_body ?? null;
  const historyStatusCopy = contributorCopy.data?.history_ui ?? null;
  const [filter, setFilter] = useState<HistoryFilter>("all");
  const counts = useMemo(() => countHistory(records), [records]);
  const visibleRecords = useMemo(
    () => filterHistory(records, filter),
    [filter, records],
  );
  const groupedRecords = useMemo(
    () => groupHistory(visibleRecords),
    [visibleRecords],
  );
  const explanations = useMemo(
    () => quarantineExplanations(records, heldRowFallback),
    [heldRowFallback, records],
  );
  return (
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="WORKSPACE / RECORD"
        title="History"
        description="What you have contributed, and what is still being reviewed."
        phase="PHASE 1"
      />
      <div className="mb-4 grid grid-cols-4 gap-3 max-[860px]:grid-cols-1">
        <StatCard
          label="In the commons"
          value={rollup ? `${rollup.all_time.accepted}` : "—"}
          detail="Server-confirmed contributions"
          tone="green"
        />
        <StatCard
          label="Held"
          value={rollup ? `${rollup.all_time.quarantined}` : "—"}
          detail="Privacy review, not rejection"
          tone="gold"
        />
        <StatCard
          label="Waiting to be scored"
          value={rollup ? `${rollup.all_time.submitted}` : "—"}
          detail="Recorded submissions"
          tone="blue"
        />
        <StatCard
          label="Final credit points"
          value={rollup ? `${rollup.credit_final.toFixed(1)}` : "—"}
          detail={
            rollup
              ? `Final: ${rollup.credit_final.toFixed(1)} · Pending: ${rollup.credit_pending.toFixed(1)} · not currency`
              : "Final and pending credit points; not currency"
          }
          tone="gold"
        />
      </div>
      {rollup?.community && <CommunityPanel standing={rollup.community} />}
      <section className="p-[26px] rounded-2xl border border-border bg-card/80">
        <div className="flex items-start justify-between gap-[18px]">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              SUBMISSIONS
            </span>
            <h2>Contribution history</h2>
          </div>
          <HistoryRefreshControl />
        </div>
        {history.state === "loading" && (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            Reading local history…
          </p>
        )}
        {history.state === "error" && (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            History unavailable. Refresh after Rust core starts.
          </p>
        )}
        {history.state === "ready" && records.length === 0 && (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            No submissions recorded on this device yet.
          </p>
        )}
        {history.state === "ready" && records.length > 0 && (
          <>
            <HistoryFilterBar
              value={filter}
              counts={counts}
              onChange={setFilter}
            />
            {visibleRecords.length === 0 ? (
              <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
                No submissions match this filter.
              </p>
            ) : (
              <div className="mt-[22px] grid gap-6">
                {groupedRecords.map((group) => (
                  <section
                    className="border-t border-border pt-[18px] first:border-t-0 first:pt-0"
                    key={group.id}
                  >
                    <div className="flex items-end justify-between gap-[18px]">
                      <div>
                        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                          PROJECT
                        </span>
                        <h3>{group.label}</h3>
                      </div>
                      <span>
                        {group.records.length} record
                        {group.records.length === 1 ? "" : "s"}
                      </span>
                    </div>
                    <div className="mt-[22px] grid gap-px border-t border-border">
                      {group.records.map((item) => (
                        <HistoryRow
                          key={item.submission_id}
                          record={item}
                          accountSignedIn={withdrawal.accountSignedIn}
                          accountStatusPending={withdrawal.accountStatusPending}
                          accountSignInPending={withdrawal.accountSignInPending}
                          accountSignInUrlAvailable={Boolean(withdrawal.accountSignInUrl)}
                          accountSignInError={withdrawal.accountSignInError}
                          openingSignInUrl={withdrawal.openingSignInUrl}
                          onAccountSignIn={withdrawal.startSignIn}
                          onOpenSignInUrl={() => void withdrawal.openSignInUrl()}
                          heldRowFallback={heldRowFallback}
                          statusCopy={historyStatusCopy}
                          onOpen={() => void detail.open(item.submission_id)}
                          confirming={
                            withdrawal.confirmingId === item.submission_id
                          }
                          busy={withdrawal.busyId === item.submission_id}
                          result={withdrawal.results[item.submission_id]}
                          error={withdrawal.errors[item.submission_id]}
                          onWithdrawRequest={() =>
                            withdrawal.request(item.submission_id)
                          }
                          onWithdrawConfirm={() =>
                            void withdrawal.confirm(item.submission_id)
                          }
                          onWithdrawCancel={withdrawal.cancel}
                        />
                      ))}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </>
        )}
      </section>
      {rollup && (
        <CreditRecordPanel
          finalPoints={rollup.credit_final}
          pendingPoints={rollup.credit_pending}
          refreshedAt={rollup.last_refreshed_at}
        />
      )}
      {history.state === "ready" &&
        rollup &&
        rollup.all_time.quarantined > 0 && (
          <section className="rounded-2xl border border-border bg-card/80 mt-4 p-[26px]">
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              PRIVACY REVIEW
            </span>
            <h2>{rollup.all_time.quarantined} held for privacy review</h2>
            <p>
              Held means not rejected. Review timing is not guaranteed, and this
              screen does not offer bulk withdrawal.
            </p>
            {explanations.map((explanation) => (
              <p
                className="m-0 text-[11px] leading-[1.55] text-muted-foreground"
                key={explanation}
              >
                {explanation}
              </p>
            ))}
            {explanations.length === 0 && !heldRowFallback && (
              <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
                Shared hold explanation unavailable.
              </p>
            )}
          </section>
        )}
      <HistoryDetailView detail={detail.data} state={detail.state} />
      {detail.data && detail.selectedId && (
        <PublicRunEditor
          key={detail.selectedId}
          submissionId={detail.selectedId}
          detail={detail.data}
        />
      )}
      {detail.data && detail.selectedId && (
        <SkillLearningPanel
          submissionId={detail.selectedId}
          detail={detail.data}
        />
      )}
    </div>
  );
}
