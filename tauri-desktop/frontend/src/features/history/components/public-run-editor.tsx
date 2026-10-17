import { useExternalUrl } from "../../../lib/tauri/use-platform-actions";
import { usePublicRun } from "../hooks/use-public-run";
import type { HistoryDetail } from "../types";
import { PublicRunForm } from "./public-run-form";
import { PublicRunPreview } from "./public-run-preview";
import { GlassButton } from "@/design-system";

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
      <p className="m-0 tc-caption tc-text-tertiary mt-4 py-[18px]">
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
    <section className="mt-4 grid gap-[18px] tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            PUBLIC WORKFLOW
          </span>
          <h2>{publication ? "Edit public page" : "Create public page"}</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
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
    <section className="mt-4 grid gap-[18px] tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            PUBLIC WORKFLOW
          </span>
          <h2>Published page</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
          Published
        </span>
      </div>
      <h3>{publication.title}</h3>
      <p>{publication.outcome_summary}</p>
      {publication.source && (
        <p className="m-0 tc-caption tc-text-tertiary">
          Varies: {publication.source.title}
        </p>
      )}
      {publication.source_unavailable && (
        <p className="m-0 tc-caption tc-text-tertiary">
          Source public run is unavailable.
        </p>
      )}
      {error && (
        <p className="tc-alert m-0">
          {error}
        </p>
      )}
      <div className="mt-3 flex flex-wrap gap-2">
        {onOpen && (
          <GlassButton
            type="button"
            onClick={onOpen}
            disabled={working}
          >
            Open page
          </GlassButton>
        )}
        <GlassButton
          type="button"
          onClick={onEdit}
          disabled={working}
        >
          Edit page
        </GlassButton>
        <GlassButton
          className="tc-text-outside"
          type="button"
          onClick={onUnpublish}
          disabled={working}
        >
          {working ? "Unpublishing…" : "Unpublish"}
        </GlassButton>
      </div>
    </section>
  );
}
