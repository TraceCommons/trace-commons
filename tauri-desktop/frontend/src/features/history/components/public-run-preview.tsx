import { Button } from "@/components/ui/button";
import type { HistoryDetail, PublicRunDraft } from "../types";

type Props = {
  detail: HistoryDetail;
  draft: PublicRunDraft;
  working: boolean;
  onEdit: () => void;
  onPublish: () => void;
};

export function PublicRunPreview({
  detail,
  draft,
  working,
  onEdit,
  onPublish,
}: Props) {
  return (
    <div className="grid gap-4">
      <h3>Exact public preview</h3>
      <PreviewField label="Page title" value={draft.title} />
      <PreviewField
        label="Creator outcome"
        value={detail.task_success ?? "Unavailable"}
      />
      <PreviewField
        label="Contributed version"
        value={detail.contributed_version}
      />
      <PreviewField label="Public outcome" value={draft.outcome_summary} />
      {draft.correction_excerpt && (
        <PreviewField
          label="Decisive correction"
          value={draft.correction_excerpt}
        />
      )}
      <PreviewField label="Use workflow" value={draft.workflow} />
      <PreviewField
        label="Reuse permission"
        value={draft.reuse_permission === "cc0_1_0" ? "CC0 1.0" : "CC BY 4.0"}
      />
      {draft.source_slug && (
        <PreviewField
          label="Source public run"
          value={`/runs/${draft.source_slug}`}
        />
      )}
      <div className="grid gap-[9px] border-t border-border pt-4">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          OBSERVED EVIDENCE
        </span>
        {draft.evidence.map((evidence) => (
          <p key={evidence.event_id}>{evidence.excerpt}</p>
        ))}
      </div>
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onEdit}
          disabled={working}
        >
          Edit draft
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onPublish}
          disabled={working}
        >
          {working
            ? "Publishing…"
            : detail.publication
              ? "Update page"
              : "Publish page"}
        </Button>
      </div>
    </div>
  );
}

function PreviewField({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-[5px] border-t border-border pt-3">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        {label}
      </span>
      <p>{value}</p>
    </div>
  );
}
