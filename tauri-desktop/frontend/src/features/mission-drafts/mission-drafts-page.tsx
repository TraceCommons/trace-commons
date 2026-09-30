import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useRef } from "react";
import { useForm } from "react-hook-form";
import { PageHeader } from "../../components/page-header";
import { ConfirmActionButton } from "../../components/confirm-action-button";
import { useContributorDisclosureCopy } from "../../lib/tauri/use-contributor-copy";
import { type MissionImportFormValues, missionImportFormSchema } from "./forms";
import { useMissionDrafts } from "./hooks/use-mission-drafts";

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
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="LOCAL / EVIDENCE"
        title="Mission drafts"
        description="Review proposed evidence-bound runs before anything is executed."
        phase="PHASE 4"
      />
      {drafts.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {drafts.error}
        </p>
      )}
      <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          DRAFT INTAKE
        </span>
        <h2>Import local proposal</h2>
        <p>
          Rust validates schema, HTTPS sources, and bounded budgets. The
          selected file is read locally and is never fetched, executed,
          published, or funded.
        </p>
        <div className="mt-6 flex gap-2.5">
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="button"
            onClick={() => input.current?.click()}
            disabled={drafts.state === "busy"}
          >
            {drafts.state === "busy" ? "Working…" : "Choose proposal"}
          </Button>
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={() => void drafts.refresh()}
            disabled={drafts.state === "busy"}
          >
            Refresh drafts
          </Button>
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
          <p className="mt-[15px] text-[11px] leading-[1.5] text-primary">
            Draft reviewed locally: {drafts.lastImport.proposal_sha256}. Curator
            review remains required.
          </p>
        )}
      </section>
      {drafts.selected && (
        <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
          <div className="flex items-start justify-between gap-[18px]">
            <div>
              <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                PROPOSAL DETAIL
              </span>
              <h2>{drafts.selected.proposal.title}</h2>
            </div>
            <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
              {drafts.selected.review.status}
            </span>
          </div>
          <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
            {drafts.selected.proposal.task}
          </p>
          <div className="my-5 flex flex-wrap gap-x-[26px] gap-y-2 text-[11px] text-muted-foreground">
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
          <div className="mt-[18px] flex justify-end gap-[9px]">
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
      <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
        <div className="flex items-start justify-between gap-[18px]">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              LOCAL INBOX
            </span>
            <h2>Mission drafts</h2>
          </div>
          <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
            {drafts.drafts.length}
          </span>
        </div>
        {drafts.state === "loading" ? (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            Reading local inbox…
          </p>
        ) : drafts.drafts.length === 0 ? (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            No local mission drafts.
          </p>
        ) : (
          <div className="mt-[22px] grid gap-px border-t border-border">
            {drafts.drafts.map((draft) => (
              <Button
                className="grid w-full grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-0 border-b border-border bg-transparent py-3.5 text-left hover:bg-muted"
                type="button"
                key={draft.id}
                onClick={() => void drafts.open(draft.id)}
                disabled={drafts.state === "busy"}
              >
                <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground">
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
              </Button>
            ))}
          </div>
        )}
      </section>
      <section className="rounded-2xl border border-border bg-card/80 p-[26px] bg-muted/70">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
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
