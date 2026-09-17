import { Button } from "@/components/ui/button";
import { CenteredNotice } from "../../../components/centered-notice";
import type { WaitingPreview } from "../types";

type WaitingReviewProps = {
  preview: WaitingPreview | null;
  state: "idle" | "loading" | "ready" | "error" | "acting";
  error: string | null;
  errorKind?: "preview" | "action" | null;
  onApprove: () => void;
  onDismiss: () => void;
  onInspect: () => void;
};

export function WaitingReview({
  preview,
  state,
  error,
  errorKind,
  onApprove,
  onDismiss,
  onInspect,
}: WaitingReviewProps) {
  if (state === "loading")
    return (
      <div className="mt-[18px] rounded-xl border border-primary/20 bg-primary/5 p-[22px]">
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Building local privacy preview…
        </p>
      </div>
    );
  if (state === "error")
    return (
      <div className="mt-[18px] rounded-xl border border-primary/20 bg-primary/5 p-[22px]">
        {errorKind === "preview" ? (
          <CenteredNotice
            tone="error"
            title="This one can't be shown."
            body={
              error ??
              "The session file changed while it was being read. Nothing has been sent, and nothing will be until it can be shown to you."
            }
          />
        ) : (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">{error}</p>
        )}
      </div>
    );
  if (!preview) return null;
  const redactionSummary =
    Object.entries(preview.redactions)
      .map(([label, count]) => `${label} ${count}`)
      .join(" · ") || "No redactions reported";
  return (
    <div className="mt-[18px] rounded-xl border border-primary/20 bg-primary/5 p-[22px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            LOCAL PREVIEW
          </span>
          <h3>What would leave this Mac</h3>
        </div>
        <span
          className={`whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start ${preview.enrolled ? "" : "bg-muted text-muted-foreground"}`}
        >
          {preview.enrolled ? "Enrolled" : "Not enrolled"}
        </span>
      </div>
      <p className="my-[18px] mb-3.5 border-l-[3px] border-chart-2 bg-background px-[15px] py-[13px] text-[13px] leading-[1.55] text-foreground">
        {preview.opening_prompt || "No opening prompt"}
      </p>
      <div className="flex flex-wrap gap-x-[18px] gap-y-2 text-[11px] text-muted-foreground">
        <span>
          <b>{formatBytes(preview.would_send_bytes)}</b> redacted payload
        </span>
        <span>
          <b>{preview.event_count}</b> events
        </span>
        <span>
          <b>{redactionSummary}</b>
        </span>
      </div>
      <p className="my-3.5 text-[11px] leading-[1.5] text-muted-foreground">
        <strong>Residual risk:</strong> {preview.residual_risk}
      </p>
      {preview.consent_scopes.length > 0 && (
        <p className="my-3.5 text-[11px] leading-[1.5] text-muted-foreground">
          Consent scopes: {preview.consent_scopes.join(" · ")}
        </p>
      )}
      <div className="mt-[18px] flex justify-end gap-[9px]">
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={onInspect}
          disabled={state === "acting"}
        >
          Look inside
        </Button>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onDismiss}
          disabled={state === "acting"}
        >
          Dismiss
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onApprove}
          disabled={state === "acting" || !preview.enrolled}
        >
          {preview.enrolled ? "Approve" : "Enroll to approve"}
        </Button>
      </div>
    </div>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
