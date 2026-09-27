import { Button } from "@/components/ui/button";
import { CenteredNotice } from "../../../components/centered-notice";
import {
  useRedactionSummary,
  useResidualSecretCopy,
} from "../../../lib/tauri/use-contributor-copy";
import type {
  EligibilityCopy,
  OutcomeCopy,
} from "../../../lib/tauri/contributor-copy-api";
import type { OutcomeVerdict, WaitingPreview } from "../types";
import { WaitingOutcomeFields } from "./waiting-outcome-fields";

type WaitingReviewProps = {
  preview: WaitingPreview | null;
  state: "idle" | "loading" | "ready" | "error" | "acting";
  error: string | null;
  errorKind?: "preview" | "action" | null;
  eligibilityCopy: EligibilityCopy | null;
  eligibilityPending: boolean;
  eligibilityError: boolean;
  outcomeCopy: OutcomeCopy | null;
  outcomeCopyPending: boolean;
  outcomeCopyError: boolean;
  verdict: OutcomeVerdict | null;
  correction: string;
  credentialRefusal: boolean;
  onVerdictChange: (verdict: OutcomeVerdict | null) => void;
  onCorrectionChange: (correction: string) => void;
  onApprove: () => void;
  onDismiss: () => void;
  onInspect: () => void;
};

export function WaitingReview({
  preview,
  state,
  error,
  errorKind,
  eligibilityCopy,
  eligibilityPending,
  eligibilityError,
  outcomeCopy,
  outcomeCopyPending,
  outcomeCopyError,
  verdict,
  correction,
  credentialRefusal,
  onVerdictChange,
  onCorrectionChange,
  onApprove,
  onDismiss,
  onInspect,
}: WaitingReviewProps) {
  const redactions = preview?.redactions ?? {};
  const distinct = preview?.redactions_distinct ?? {};
  const redactionSummary = useRedactionSummary(
    redactions,
    distinct,
    preview !== null,
  );
  const residualCopy = useResidualSecretCopy(redactions);
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
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            {error}
          </p>
        )}
      </div>
    );
  if (!preview) return null;
  const eligibilityRequired = Boolean(preview.entry.eligibility);
  const eligibilityReady = !eligibilityRequired || eligibilityCopy !== null;
  const canApprove =
    !eligibilityRequired || eligibilityCopy?.can_contribute === true;
  const residualRequired = Object.keys(preview.redactions).some((label) =>
    label.startsWith("residual_secret_at:"),
  );
  const redactionCopyReady = Boolean(redactionSummary.data);
  const residualCopyReady = !residualRequired || Boolean(residualCopy.data);
  return (
    <div className="mt-[18px] rounded-xl border border-primary/20 bg-primary/5 p-[22px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            LOCAL PREVIEW
          </span>
          <h3>What would leave this computer</h3>
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
      </div>
      {redactionSummary.isPending && (
        <p className="my-3.5 text-[11px] text-muted-foreground">
          Preparing redaction summary…
        </p>
      )}
      {redactionSummary.isError && (
        <p className="my-3.5 text-[11px] text-destructive">
          Redaction summary unavailable. Contribution is disabled.
        </p>
      )}
      {redactionSummary.data && (
        <div className="my-3.5 grid gap-3 text-[11px] leading-[1.5]">
          <div className="grid gap-1">
            <strong>Removed</strong>
            {redactionSummary.data.removed.length === 0 ? (
              <p className="m-0 text-muted-foreground">
                Nothing matched for removal.
              </p>
            ) : (
              redactionSummary.data.removed.map((item) => (
                <p className="m-0 text-muted-foreground" key={item.family}>
                  <strong className="text-foreground">
                    {item.occurrences} {item.display}
                    {item.distinct > 0 && item.distinct < item.occurrences
                      ? ` (${item.distinct} distinct)`
                      : ""}
                  </strong>
                  {`: ${item.description}`}
                  {item.detail.length > 0 && ` (${item.detail.join(", ")})`}
                </p>
              ))
            )}
          </div>
          {redactionSummary.data.still_present.length > 0 && (
            <div className="grid gap-1 rounded-md border border-destructive/40 bg-destructive/5 p-3">
              <strong className="text-destructive">
                Found, and still in what would be sent
              </strong>
              {redactionSummary.data.still_present.map((item) => (
                <p className="m-0 text-muted-foreground" key={item.family}>
                  <strong className="text-foreground">
                    {item.occurrences} {item.display}
                  </strong>
                  {`: ${item.description}`}
                  {item.detail.length > 0 && ` (${item.detail.join(", ")})`}
                </p>
              ))}
            </div>
          )}
        </div>
      )}
      {residualRequired && (
        <p className="my-3.5 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-[11px] leading-[1.5] text-destructive">
          {residualCopy.data ??
            (residualCopy.isError
              ? "Residual secret warning unavailable. Contribution is disabled."
              : "Loading residual secret warning…")}
        </p>
      )}
      <p className="my-3.5 text-[11px] leading-[1.5] text-muted-foreground">
        <strong>Residual risk:</strong> {preview.residual_risk}
      </p>
      <p className="my-3.5 text-[11px] leading-[1.5] text-muted-foreground">
        {preview.gate_statement}
      </p>
      {preview.consent_scopes.length > 0 && (
        <p className="my-3.5 text-[11px] leading-[1.5] text-muted-foreground">
          Consent scopes: {preview.consent_scopes.join(" · ")}
        </p>
      )}
      {eligibilityRequired && (
        <div className="my-3.5 grid gap-1 text-[11px] leading-[1.5] text-muted-foreground">
          {eligibilityCopy ? (
            <>
              <p className="m-0">{eligibilityCopy.state_line}</p>
              {eligibilityCopy.reason_line && (
                <p className="m-0">{eligibilityCopy.reason_line}</p>
              )}
            </>
          ) : eligibilityError ? (
            <p className="m-0 text-destructive">
              Contribution eligibility could not be checked. Approval is disabled.
            </p>
          ) : eligibilityPending ? (
            <p className="m-0">Checking contribution eligibility…</p>
          ) : null}
        </div>
      )}
      {outcomeCopy ? (
        <WaitingOutcomeFields
          copy={outcomeCopy}
          verdict={verdict}
          correction={correction}
          disabled={state === "acting"}
          onVerdictChange={onVerdictChange}
          onCorrectionChange={onCorrectionChange}
        />
      ) : (
        <p className="my-3.5 text-[11px] text-destructive">
          {outcomeCopyError
            ? "Outcome and correction disclosure unavailable. Contribution is disabled."
            : outcomeCopyPending
              ? "Loading outcome and correction disclosure…"
              : "Outcome and correction disclosure unavailable."}
        </p>
      )}
      {credentialRefusal && outcomeCopy && (
        <div className="my-3.5 rounded-md border border-destructive/40 bg-destructive/5 p-3 text-[11px] leading-[1.5]">
          <strong className="text-destructive">
            {outcomeCopy.correction_credential_headline}
          </strong>
          <p className="m-0 mt-1 text-muted-foreground">
            {outcomeCopy.correction_credential_body}
          </p>
        </div>
      )}
      {error && errorKind === "action" && !credentialRefusal && (
        <p className="my-3.5 text-[11px] text-destructive">{error}</p>
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
          disabled={
            state === "acting" ||
            !preview.enrolled ||
            !canApprove ||
            !eligibilityReady ||
            !outcomeCopy ||
            !redactionCopyReady ||
            !residualCopyReady
          }
        >
          {!preview.enrolled
            ? "Enroll to approve"
            : eligibilityRequired && eligibilityCopy && !canApprove
              ? "Not eligible"
              : eligibilityRequired && !eligibilityReady
                ? "Checking eligibility…"
                : "Contribute"}
        </Button>
      </div>
    </div>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
