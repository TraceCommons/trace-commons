import { useState } from "react";
import { Button } from "@/components/ui/button";
import { CenteredNotice } from "../../components/centered-notice";
import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import type { CoreStatus } from "../../lib/tauri/types";
import { useProjects, useSettings } from "../settings/public";
import { ArmingOffer } from "./components/arming-offer";
import { CertificatePanel } from "./components/certificate-panel";
import { PreviewInspector } from "./components/preview-inspector";
import { PrivateInferenceOffer } from "./components/private-inference-offer";
import { QueueOutcomeDisclosure } from "./components/queue-outcome-disclosure";
import { QueueStatusPanel } from "./components/queue-status-panel";
import { UndoBar } from "./components/undo-bar";
import { WaitingProjectFolder } from "./components/waiting-project-folder";
import { WaitingProjectGroup } from "./components/waiting-project-group";
import { WaitingReview } from "./components/waiting-review";
import { useArmingOffer } from "./hooks/use-arming-offer";
import { useCertificateCopy } from "./hooks/use-certificate-copy";
import { usePrivateInferenceOffer } from "./hooks/use-private-inference-offer";
import { useQueueOutcomeCounts } from "./hooks/use-queue-outcome-counts";
import { useWaitingBulkApproval } from "./hooks/use-waiting-bulk-approval";
import { useWaitingData } from "./hooks/use-waiting-data";
import { useWaitingReview } from "./hooks/use-waiting-review";
import { useWaitingUndo } from "./hooks/use-waiting-undo";

export function WaitingPage({ status }: { status: CoreStatus | null }) {
  const waiting = useWaitingData();
  const undo = useWaitingUndo();
  const review = useWaitingReview(undo.prepare);
  const bulk = useWaitingBulkApproval(undo.prepare);
  const arming = useArmingOffer();
  const privateInference = usePrivateInferenceOffer();
  const outcomes = useQueueOutcomeCounts();
  const settings = useSettings();
  const projects = useProjects();
  const evidenceAdmitted = settings.data?.admission_evidence_required === true;
  const certificateCopy = useCertificateCopy(evidenceAdmitted);
  const [inspecting, setInspecting] = useState(false);
  const entries = waiting.data?.pending ?? [];
  const [openProjectId, setOpenProjectId] = useState<string | null>(null);
  const groups = Array.from(
    entries
      .reduce((map, entry) => {
        const group = map.get(entry.project_id) ?? {
          label: entry.project_label,
          entries: [] as typeof entries,
        };
        group.entries.push(entry);
        map.set(entry.project_id, group);
        return map;
      }, new Map<string, { label: string; entries: typeof entries }>())
      .entries(),
  );
  const openGroup = groups.find(([projectId]) => projectId === openProjectId);
  return (
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="WORKSPACE / REVIEW"
        title="Waiting"
        description="Nothing is sent unless you say so."
        phase="PHASE 1"
      />
      <div className="mb-4 grid grid-cols-3 gap-3 max-[860px]:grid-cols-1">
        <StatCard
          label="Needs review"
          value={waiting.state === "ready" ? `${entries.length}` : "—"}
          detail="Pending local decisions"
        />
        <StatCard
          label="Privacy boundary"
          value="Local"
          detail="Content stays on this machine"
          tone="blue"
        />
        <StatCard
          label="Daemon"
          value={waiting.state === "error" ? "Offline" : "Watching"}
          detail="Existing Rust contributor core"
          tone={waiting.state === "error" ? "gold" : "green"}
        />
      </div>
      {status && (
        <QueueStatusPanel
          health={status.daemon.health}
          budget={status.daemon.daily_budget}
          routing={status.daemon.routing}
        />
      )}
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
      <UndoBar
        scope={undo.scope}
        seconds={undo.seconds}
        busy={undo.busy}
        error={undo.error}
        onUndo={() => void undo.undo()}
        onDismiss={undo.dismiss}
      />
      <CertificatePanel entries={entries} copy={certificateCopy.data ?? null} />
      <QueueOutcomeDisclosure
        reasons={outcomes.data?.reasons ?? null}
        lines={outcomes.data?.lines ?? {}}
      />
      <section className="p-[26px] rounded-2xl border border-border bg-card/80">
        <div className="flex items-start justify-between gap-[18px]">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              QUEUE
            </span>
            <h2>Sessions awaiting your decision</h2>
          </div>
          <Button
            className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
            type="button"
            onClick={() =>
              void Promise.all([waiting.refresh(), outcomes.refresh()])
            }
            disabled={
              waiting.state === "loading" || outcomes.state === "loading"
            }
          >
            Refresh
          </Button>
        </div>
        {waiting.state === "loading" && (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            Reading local queue…
          </p>
        )}
        {waiting.state === "error" && (
          <CenteredNotice
            title="The watcher isn't running."
            body="It didn't answer. Nothing is being noticed or sent while it's stopped, and sessions already waiting stay on this machine."
          />
        )}
        {waiting.state === "ready" && entries.length === 0 && (
          <CenteredNotice
            title="Nothing is waiting."
            body="When a session finishes and goes quiet, it shows up here. Nothing is sent unless you say so."
          />
        )}
        {waiting.state === "ready" &&
          entries.length > 0 &&
          (openGroup ? (
            <div className="block">
              <Button
                className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
                type="button"
                onClick={() => {
                  setOpenProjectId(null);
                  review.clear();
                }}
              >
                ‹ All projects
              </Button>
              <WaitingProjectGroup
                projectId={openGroup[0]}
                label={openGroup[1].label}
                entries={openGroup[1].entries}
                selectedId={review.selectedId}
                busy={bulk.busyId === openGroup[0]}
                message={bulk.messages[openGroup[0]]}
                showSubmitAll={false}
                onReview={(entryId) => void review.review(entryId)}
                onSubmitAll={(id) => void bulk.approve(id, openGroup[1].label)}
                onSubmitAllAs={(id, outcome) =>
                  void bulk.approve(id, openGroup[1].label, outcome)
                }
              />
            </div>
          ) : (
            <div className="mt-[22px] grid gap-2.5">
              {groups.map(([projectId, group]) => (
                <WaitingProjectFolder
                  key={projectId}
                  projectId={projectId}
                  label={group.label}
                  path={group.entries[0]?.project_path}
                  count={group.entries.length}
                  entries={group.entries}
                  busy={bulk.busyId === projectId || projects.state === "busy"}
                  message={bulk.messages[projectId]}
                  onOpen={(id) => {
                    review.clear();
                    setOpenProjectId(id);
                  }}
                  onSubmitAll={(id) => void bulk.approve(id, group.label)}
                  onSubmitAllAs={(id, outcome) =>
                    void bulk.approve(id, group.label, outcome)
                  }
                  onIgnore={async (id) => {
                    await projects.setMode(id, "ignore");
                    await waiting.refresh();
                  }}
                />
              ))}
            </div>
          ))}
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
          onDismiss={() => void review.dismiss()}
          onInspect={() => setInspecting(true)}
        />
        <PreviewInspector
          preview={review.preview}
          open={inspecting}
          onClose={() => setInspecting(false)}
          onReviewed={() => {
            void review.refetch();
            void waiting.refresh();
          }}
        />
      </section>
    </div>
  );
}
