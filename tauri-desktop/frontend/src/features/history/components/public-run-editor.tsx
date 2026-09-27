import { Button } from "@/components/ui/button";
import { useExternalUrl } from "../../../lib/tauri/use-platform-actions";
import { usePublicRun } from "../hooks/use-public-run";
import type { HistoryDetail } from "../types";
import { PublicRunForm } from "./public-run-form";
import { PublicRunPreview } from "./public-run-preview";

export function PublicRunEditor({
  submissionId,
  detail,
}: {
  submissionId: string;
  detail: HistoryDetail;
}) {
  const publication = detail.publication;
  const externalUrl = useExternalUrl();
  const run = usePublicRun(submissionId, detail);
  const eligible =
    detail.contribution_status === "accepted" && detail.task_success !== null;
  if (!eligible && !publication)
    return (
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground mt-4 py-[18px]">
        A public page becomes available after this contribution is accepted.
      </p>
    );
  if (publication && !run.editing && !run.draft)
    return (
      <PublishedPage
        publication={publication}
        working={run.working || externalUrl.isPending}
        error={run.error}
        onOpen={() =>
          publication.public_url
            ? void externalUrl.open(publication.public_url)
            : undefined
        }
        onEdit={run.beginEdit}
        onUnpublish={() => void run.unpublish()}
      />
    );
  return (
    <section className="mt-4 grid gap-[18px] rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            PUBLIC WORKFLOW
          </span>
          <h2>{publication ? "Edit public page" : "Create public page"}</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          Separate consent
        </span>
      </div>
      {run.draft ? (
        <PublicRunPreview
          detail={detail}
          draft={run.draft}
          working={run.working}
          onEdit={() => {
            run.beginEdit();
          }}
          onPublish={() => void run.publish()}
        />
      ) : (
        <PublicRunForm
          detail={detail}
          working={run.working}
          error={run.error}
          editingPublished={Boolean(publication)}
          onReview={(input) => void run.review(input)}
          onCancel={run.cancelEdit}
        />
      )}
    </section>
  );
}

function PublishedPage({
  publication,
  working,
  error,
  onOpen,
  onEdit,
  onUnpublish,
}: {
  publication: NonNullable<HistoryDetail["publication"]>;
  working: boolean;
  error: string | null;
  onOpen?: () => void;
  onEdit: () => void;
  onUnpublish: () => void;
}) {
  return (
    <section className="mt-4 grid gap-[18px] rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            PUBLIC WORKFLOW
          </span>
          <h2>Published page</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          Published
        </span>
      </div>
      <h3>{publication.title}</h3>
      <p>{publication.outcome_summary}</p>
      {publication.source && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Varies: {publication.source.title}
        </p>
      )}
      {publication.source_unavailable && (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Source public run is unavailable.
        </p>
      )}
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive m-0">
          {error}
        </p>
      )}
      <div className="mt-6 flex gap-2.5">
        {onOpen && (
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={onOpen}
            disabled={working}
          >
            Open page
          </Button>
        )}
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onEdit}
          disabled={working}
        >
          Edit page
        </Button>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
          type="button"
          onClick={onUnpublish}
          disabled={working}
        >
          {working ? "Unpublishing…" : "Unpublish"}
        </Button>
      </div>
    </section>
  );
}
