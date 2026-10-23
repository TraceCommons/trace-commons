import { zodResolver } from "@hookform/resolvers/zod";
import { useRef } from "react";
import { useForm } from "react-hook-form";
import { PageHeader } from "../../components/page-header";
import { ConfirmActionButton } from "../../components/confirm-action-button";
import { useContributorDisclosureCopy } from "../../lib/tauri/use-contributor-copy";
import { type MissionImportFormValues, missionImportFormSchema } from "./forms";
import { useMissionDrafts } from "./hooks/use-mission-drafts";
import { ButtonPrimary, GlassButton, Input } from "@/design-system";

export function MissionDraftsPage() {
  const drafts = useMissionDrafts();
  const contributorCopy = useContributorDisclosureCopy();
  const deleteCopy = contributorCopy.data?.mission_drafts_ui;
  const input = useRef<HTMLInputElement>(null);
  const form = useForm<MissionImportFormValues>({
    resolver: zodResolver(missionImportFormSchema),
  });
  const fileField = form.register("file");
  return (
    <div className="tc-page">
      <PageHeader
        title="Missions"
        description="Review proposed evidence-bound runs before anything is executed."
        titleHidden
      />
      {drafts.error && (
        <p className="tc-alert">
          {drafts.error}
        </p>
      )}
      <section className="tc-card mb-2.5">
        <span className="mb-1.5 block tc-eyebrow">
          DRAFT INTAKE
        </span>
        <h2>Import local proposal</h2>
        <p>
          Rust validates schema, HTTPS sources, and bounded budgets. The
          selected file is read locally and is never fetched, executed,
          published, or funded.
        </p>
        <div className="mt-3 flex flex-wrap gap-2">
          <ButtonPrimary size="sm"
            type="button"
            onClick={() => input.current?.click()}
            disabled={drafts.state === "busy"}
          >
            {drafts.state === "busy" ? "Working…" : "Choose proposal"}
          </ButtonPrimary>
          <GlassButton
            type="button"
            onClick={() => void drafts.refresh()}
            disabled={drafts.state === "busy"}
          >
            Refresh drafts
          </GlassButton>
        </div>
        <Input
          {...fileField}
          ref={(element) => {
            fileField.ref(element);
            input.current = element;
          }}
          className="absolute h-px w-px overflow-hidden whitespace-nowrap [clip:rect(0_0_0_0)] [clip-path:inset(50%)]"
          type="file"
          accept="application/json"
          onChange={(event) => {
            const files = event.currentTarget.files;
            form.setValue("file", files, { shouldDirty: true });
            const file = files?.[0];
            form.resetField("file");
            event.currentTarget.value = "";
            if (file) void drafts.importFile(file);
          }}
        />
        {drafts.lastImport && (
          <p className="mt-2 tc-caption tc-text-accent">
            Draft reviewed locally: {drafts.lastImport.proposal_sha256}. Curator
            review remains required.
          </p>
        )}
      </section>
      {drafts.selected && (
        <section className="tc-card mb-2.5">
          <div className="flex items-start justify-between gap-3">
            <div>
              <span className="mb-1.5 block tc-eyebrow">
                PROPOSAL DETAIL
              </span>
              <h2>{drafts.selected.proposal.title}</h2>
            </div>
            <span className="tc-chip tc-chip--glass self-start">
              {drafts.selected.review.status}
            </span>
          </div>
          <p className="m-0 tc-caption tc-text-tertiary">
            {drafts.selected.proposal.task}
          </p>
          <div className="my-2.5 flex flex-wrap gap-x-4 gap-y-1.5 tc-caption tc-text-tertiary">
            <span>
              <b>Author</b>
              {drafts.selected.proposal.author_id}
            </span>
            <span>
              <b>Evaluator</b>
              {drafts.selected.proposal.evaluator_id}
            </span>
            <span>
              <b>Rubric</b>
              {drafts.selected.proposal.rubric_version}
            </span>
          </div>
          <div className="mt-[22px] grid grid-cols-2 gap-4">
            <div>
              <b>Claim</b>
              <p>{drafts.selected.proposal.claim_to_test}</p>
            </div>
            <div>
              <b>Sources</b>
              <p>{drafts.selected.proposal.source_urls.join(" · ")}</p>
            </div>
            <div>
              <b>Budget</b>
              <p>
                {drafts.selected.proposal.budget.max_duration_seconds}s ·{" "}
                {drafts.selected.proposal.budget.max_input_tokens} input ·{" "}
                {drafts.selected.proposal.budget.max_output_tokens} output
              </p>
            </div>
            <div>
              <b>Required evidence</b>
              <p>{drafts.selected.proposal.required_evidence.join(" · ")}</p>
            </div>
          </div>
          <div className="mt-3 flex justify-end gap-2">
            <ConfirmActionButton
              label={deleteCopy?.delete}
              title={deleteCopy?.delete_confirm_title}
              description={deleteCopy?.delete_confirm}
              confirmLabel={deleteCopy?.delete}
              cancelLabel={deleteCopy?.cancel}
              workingLabel={deleteCopy?.working}
              unavailableLabel="Delete confirmation copy unavailable. Reload before deleting."
              busy={drafts.state === "busy"}
              onConfirm={() => void drafts.remove()}
            />
          </div>
        </section>
      )}
      <section className="tc-card">
        <div className="flex items-start justify-between gap-3">
          <div>
            <span className="mb-1.5 block tc-eyebrow">
              LOCAL INBOX
            </span>
            <h2>Mission drafts</h2>
          </div>
          <span className="tc-chip tc-chip--glass self-start">
            {drafts.drafts.length}
          </span>
        </div>
        {drafts.state === "loading" ? (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
            Reading local inbox…
          </p>
        ) : drafts.drafts.length === 0 ? (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
            No local mission drafts.
          </p>
        ) : (
          <div className="mt-3 grid gap-px">
            {drafts.drafts.map((draft) => (
              <button
                className="grid w-full grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-lg border-0 bg-transparent px-1.5 py-2 text-left text-inherit hover:bg-white/5 tc-hairline-bottom"
                type="button"
                key={draft.id}
                onClick={() => void drafts.open(draft.id)}
                disabled={drafts.state === "busy"}
              >
                <span className="tc-tool-tile tc-tool-tile--lg">
                  M
                </span>
                <span className="grid min-w-0 gap-1">
                  <strong>{draft.id}</strong>
                  <span>{draft.source_count} declared sources</span>
                </span>
                <span className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
                  <strong>{draft.status}</strong>
                  <span>Review required</span>
                </span>
              </button>
            ))}
          </div>
        )}
      </section>
      <section className="tc-card bg-tc-tint/70">
        <span className="mb-1.5 block tc-eyebrow">
          AUTHORITY
        </span>
        <h2>Review is not acceptance</h2>
        <p>
          A draft can declare criteria, sources, tools, models, and budgets. It
          does not establish task acceptance, attribution, or verified outcome.
        </p>
      </section>
    </div>
  );
}
