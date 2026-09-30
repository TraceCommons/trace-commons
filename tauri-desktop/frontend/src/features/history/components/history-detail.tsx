import type { HistoryDetail } from "../types";

export function HistoryDetailView({
  detail,
  state,
}: {
  detail: HistoryDetail | null;
  state: "idle" | "loading" | "ready" | "error";
}) {
  if (state === "loading")
    return (
      <section className="tc-card mt-2.5">
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Reading owned session detail…
        </p>
      </section>
    );
  if (state === "error")
    return (
      <section className="tc-card mt-2.5">
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Detail unavailable. Account session may be required.
        </p>
      </section>
    );
  if (!detail) return null;
  return (
    <section className="tc-card mt-2.5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            SESSION DETAIL
          </span>
          <h2>{detail.contribution_status ?? "Contribution"}</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
          Account-gated
        </span>
      </div>
      {detail.content_unavailable ? (
        <p className="m-0 tc-caption tc-text-tertiary">
          Content is unavailable from server. Metadata and bounded evidence
          remain visible.
        </p>
      ) : (
        <p className="m-0 tc-caption tc-text-tertiary">
          {detail.task ?? "No task description supplied."}
        </p>
      )}
      <div className="my-2.5 flex flex-wrap gap-x-4 gap-y-1.5 tc-caption tc-text-tertiary">
        <span>
          <b>Outcome</b>
          {outcomeLabel(detail.task_success)}
        </span>
        <span>
          <b>Version</b>
          {detail.contributed_version}
        </span>
        <span>
          <b>Consent policy</b>
          {detail.consent_policy_version}
        </span>
        <span>
          <b>Redaction</b>
          {detail.redaction_pipeline_version}
        </span>
      </div>
      {detail.permitted_uses.length > 0 && (
        <div className="mt-5 border-t border-border pt-4">
          <span className="mb-1.5 block tc-eyebrow">
            PERMITTED USES
          </span>
          <p>
            {detail.permitted_uses
              .map((use) => use.replaceAll("_", " "))
              .join(" · ")}
          </p>
        </div>
      )}
      {detail.human_correction && (
        <div className="mt-5 border-t border-border pt-4">
          <span className="mb-1.5 block tc-eyebrow">
            DECISIVE CORRECTION
          </span>
          <p>{detail.human_correction}</p>
        </div>
      )}
      {detail.evidence.length > 0 && (
        <div className="mt-3 pt-3 tc-hairline-top">
          <span className="mb-1.5 block tc-eyebrow">
            BOUNDED EVIDENCE
          </span>
          {detail.evidence.map((item) => (
            <div
              className="grid grid-cols-[150px_minmax(0,1fr)] gap-3 tc-hairline-top py-2 tc-caption tc-text-tertiary"
              key={item.event_id}
            >
              <span>{item.kind}</span>
              <span>{item.excerpt}</span>
            </div>
          ))}
        </div>
      )}
      {detail.publication && (
        <p className="mt-2 tc-caption tc-text-accent">
          Published page: {detail.publication.title}
        </p>
      )}
    </section>
  );
}

function outcomeLabel(value: string | null) {
  if (value === "success") return "Completed";
  if (value === "partial") return "Partly completed";
  if (value === "failure") return "Did not complete";
  return value ?? "Unavailable";
}
