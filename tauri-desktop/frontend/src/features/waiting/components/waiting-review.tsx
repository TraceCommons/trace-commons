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
import { ButtonPrimary, GlassButton, TertiaryLink } from "@/design-system";

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
      <div className="mt-2.5 tc-card tc-card--quiet">
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Building local privacy preview…
        </p>
      </div>
    );
  if (state === "error")
    return (
      <div className="mt-2.5 tc-card tc-card--quiet">
        {errorKind === "preview" ? (
          <CenteredNotice
            tone="error"
            title="This one can't be shown."
            body={
              error ??
              "The trace file changed while it was being read. Nothing has been sent, and nothing will be until it can be shown to you."
            }
          />
        ) : (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
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
    <div className="mt-2.5 tc-card tc-card--quiet">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            LOCAL PREVIEW
          </span>
          <h3>What would leave this computer</h3>
        </div>
        <span
          className={`tc-chip tc-chip--glass self-start ${preview.enrolled ? "" : "bg-tc-tint text-tc-secondary"}`}
        >
          {preview.enrolled ? "Enrolled" : "Not enrolled"}
        </span>
      </div>
      <p className="tc-card tc-card--quiet my-[18px] mb-3.5 text-[13px] leading-[1.55] text-tc-primary">
        {preview.opening_prompt || "No opening prompt"}
      </p>
      <div className="flex flex-wrap gap-x-[18px] gap-y-2 text-[11px] text-tc-secondary">
        <span>
          <b>{formatBytes(preview.would_send_bytes)}</b> redacted payload
        </span>
        <span>
          <b>{preview.event_count}</b> events
        </span>
      </div>
      {redactionSummary.isPending && (
        <p className="my-3.5 text-[11px] text-tc-secondary">
          Preparing redaction summary…
        </p>
      )}
      {redactionSummary.isError && (
        <p className="my-2 tc-caption tc-text-outside">
          Redaction summary unavailable. Contribution is disabled.
        </p>
      )}
      {redactionSummary.data && (
        <div className="my-3.5 grid gap-3 text-[11px] leading-[1.5]">
          <div className="grid gap-1">
            <strong>Removed</strong>
            {redactionSummary.data.removed.length === 0 ? (
              <p className="m-0 text-tc-secondary">
                Nothing matched for removal.
              </p>
            ) : (
              redactionSummary.data.removed.map((item) => (
                <p className="m-0 text-tc-secondary" key={item.family}>
                  <strong className="text-[var(--tc-text-primary)]">
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
            <div className="tc-card tc-card--quiet grid gap-1 border-tc-outside/40">
              <strong className="text-tc-outside">
                Found, and still in what would be sent
              </strong>
              {redactionSummary.data.still_present.map((item) => (
                <p className="m-0 text-tc-secondary" key={item.family}>
                  <strong className="text-[var(--tc-text-primary)]">
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
        <p className="tc-card tc-card--quiet my-3.5 border-tc-outside/40 text-[11px] leading-[1.5] text-tc-outside">
          {residualCopy.data ??
            (residualCopy.isError
              ? "Residual secret warning unavailable. Contribution is disabled."
              : "Loading residual secret warning…")}
        </p>
      )}
      <p className="my-2 tc-caption tc-text-tertiary">
        <strong>Residual risk:</strong> {preview.residual_risk}
      </p>
      <p className="my-2 tc-caption tc-text-tertiary">
        {preview.gate_statement}
      </p>
      {preview.consent_scopes.length > 0 && (
        <p className="my-2 tc-caption tc-text-tertiary">
          Consent scopes: {preview.consent_scopes.join(" · ")}
        </p>
      )}
      {eligibilityRequired && (
        <div className="my-3.5 grid gap-1 text-[11px] leading-[1.5] text-tc-secondary">
          {eligibilityCopy ? (
            <>
              <p className="m-0">{eligibilityCopy.state_line}</p>
              {eligibilityCopy.reason_line && (
                <p className="m-0">{eligibilityCopy.reason_line}</p>
              )}
            </>
          ) : eligibilityError ? (
            <p className="m-0 text-tc-outside">
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
        <p className="my-2 tc-caption tc-text-outside">
          {outcomeCopyError
            ? "Outcome and correction disclosure unavailable. Contribution is disabled."
            : outcomeCopyPending
              ? "Loading outcome and correction disclosure…"
              : "Outcome and correction disclosure unavailable."}
        </p>
      )}
      {credentialRefusal && outcomeCopy && (
        <div className="tc-card tc-card--quiet my-3.5 border-tc-outside/40 text-[11px] leading-[1.5]">
          <strong className="text-tc-outside">
            {outcomeCopy.correction_credential_headline}
          </strong>
          <p className="m-0 mt-1 text-tc-secondary">
            {outcomeCopy.correction_credential_body}
          </p>
        </div>
      )}
      {error && errorKind === "action" && !credentialRefusal && (
        <p className="my-2 tc-caption tc-text-outside">{error}</p>
      )}
      <div className="mt-3 flex justify-end gap-2">
        <TertiaryLink
          type="button"
          onClick={onInspect}
          disabled={state === "acting"}
        >
          Look inside
        </TertiaryLink>
        <GlassButton
          type="button"
          onClick={onDismiss}
          disabled={state === "acting"}
        >
          Dismiss
        </GlassButton>
        <ButtonPrimary size="sm"
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
        </ButtonPrimary>
      </div>
    </div>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
