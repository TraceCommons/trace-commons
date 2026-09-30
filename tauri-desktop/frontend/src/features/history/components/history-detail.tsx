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
      <section className="rounded-2xl border border-border bg-card/80 mt-4 p-[26px]">
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Reading owned session detail…
        </p>
      </section>
    );
  if (state === "error")
    return (
      <section className="rounded-2xl border border-border bg-card/80 mt-4 p-[26px]">
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Detail unavailable. Account session may be required.
        </p>
      </section>
    );
  if (!detail) return null;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mt-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            SESSION DETAIL
          </span>
          <h2>{detail.contribution_status ?? "Contribution"}</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          Account-gated
        </span>
      </div>
      {detail.content_unavailable ? (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Content is unavailable from server. Metadata and bounded evidence
          remain visible.
        </p>
      ) : (
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          {detail.task ?? "No task description supplied."}
        </p>
      )}
      <div className="my-5 flex flex-wrap gap-x-[26px] gap-y-2 text-[11px] text-muted-foreground">
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
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
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
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            DECISIVE CORRECTION
          </span>
          <p>{detail.human_correction}</p>
        </div>
      )}
      {detail.evidence.length > 0 && (
        <div className="mt-6 border-t border-border pt-5">
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            BOUNDED EVIDENCE
          </span>
          {detail.evidence.map((item) => (
            <div
              className="grid grid-cols-[150px_minmax(0,1fr)] gap-3 border-t border-border py-[9px] text-[11px] text-muted-foreground"
              key={item.event_id}
            >
              <span>{item.kind}</span>
              <span>{item.excerpt}</span>
            </div>
          ))}
        </div>
      )}
      {detail.publication && (
        <p className="mt-[15px] text-[11px] leading-[1.5] text-primary">
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
