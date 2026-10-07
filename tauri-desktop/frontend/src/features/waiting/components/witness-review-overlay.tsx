import type { UseMutationResult } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import {
  useRedactionSummary,
  useResidualSecretCopy,
  useWitnessReviewCopy,
} from "../../../lib/tauri/use-contributor-copy";
import type { WitnessReview } from "../api/native-review-api";
import { ButtonPrimary, Checkbox, GlassButton } from "@/design-system";

type WitnessMutation = UseMutationResult<WitnessReview, Error, boolean, unknown>;

export function WitnessReviewOverlay({
  open,
  onOpenChange,
  mutation,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mutation: WitnessMutation;
}) {
  const copy = useWitnessReviewCopy();
  const [confirmed, setConfirmed] = useState(false);
  useEffect(() => {
    if (!open) setConfirmed(false);
  }, [open]);
  // A busy witness judged nothing: it is not a refusal, and the person may
  // send the review again once the witness has room.
  const busy =
    mutation.isSuccess && !mutation.data.ready && mutation.data.state === "Busy"
      ? mutation.data
      : null;
  const error = mutation.isError
    ? (copy.data?.failed ??
      "Witness review result could not be confirmed. Check the entry's current state before retrying.")
    : mutation.isSuccess && !mutation.data.ready && !busy
      ? (mutation.data.message ?? "Witness review was not confirmed.")
      : null;
  const redactions = mutation.data?.ready ? mutation.data.summary.redactions : {};
  const redactionSummary = useRedactionSummary(redactions, {}, mutation.data?.ready === true);
  const residualCopy = useResidualSecretCopy(redactions);
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title={copy.data?.heading ?? "Witness review"}
      description={copy.data?.disclosure}
      footer={
        <div className="flex justify-end gap-2">
          <GlassButton
            type="button"
            onClick={() => onOpenChange(false)}
          >
            {copy.data?.cancel ?? "Cancel"}
          </GlassButton>
          <ButtonPrimary size="sm"
            type="button"
            onClick={() => mutation.mutate(true)}
            disabled={
              mutation.isPending ||
              (mutation.isSuccess && !busy) ||
              !copy.data ||
              !confirmed
            }
          >
            {mutation.isPending
              ? "Reviewing…"
              : (copy.data?.confirm ?? "Confirm witness review")}
          </ButtonPrimary>
        </div>
      }
    >
      <div className="grid gap-3 text-[12px] text-tc-secondary">
        {copy.isPending && <p className="m-0">Loading review disclosure…</p>}
        {copy.isError && (
          <p className="m-0 text-tc-outside">
            Disclosure unavailable. Witness review is disabled.
          </p>
        )}
        {copy.data && <p className="m-0">{copy.data.immutable}</p>}
        {copy.data && (
          <label className="tc-card tc-card--quiet flex items-start gap-2.5 text-tc-primary">
            <Checkbox
              checked={confirmed}
              onChange={(value) => setConfirmed(value === true)}
              disabled={mutation.isPending || (mutation.isSuccess && !busy)}
              aria-label="Confirm sending unredacted session to witness"
            />
            <span>I understand and want to send this session for review.</span>
          </label>
        )}
        {mutation.isPending && copy.data && (
          <p className="m-0">{copy.data.working}</p>
        )}
        {busy && (
          <div
            role="status"
            className="tc-card tc-card--quiet grid gap-1 text-tc-primary"
          >
            <p className="m-0">{busy.message ?? copy.data?.failed_busy}</p>
            {busy.retryLine && <p className="m-0">{busy.retryLine}</p>}
          </div>
        )}
        {error && (
          <p className="tc-card tc-card--quiet m-0 border-tc-outside/30 text-tc-outside">
            {error}
          </p>
        )}
        {mutation.isSuccess && mutation.data.ready && (
          <div className="grid gap-2 text-tc-accent">
            <p className="m-0">
              Reviewed envelope:{" "}
              {mutation.data.summary.would_send_bytes.toLocaleString()} bytes ·{" "}
              {mutation.data.summary.event_count.toLocaleString()} events
            </p>
            {redactionSummary.data?.removed.map((item) => (
              <p className="m-0" key={item.family}>
                {item.occurrences} {item.display}: {item.description}
              </p>
            ))}
            {redactionSummary.data?.still_present.map((item) => (
              <p className="m-0 text-tc-outside" key={item.family}>
                {item.occurrences} {item.display} remain: {item.description}
              </p>
            ))}
            {redactionSummary.isError && (
              <p className="m-0 text-tc-outside">
                Redaction details unavailable.
              </p>
            )}
            {residualCopy.data && (
              <p className="m-0 text-tc-outside">{residualCopy.data}</p>
            )}
            {residualCopy.isError && !residualCopy.data && (
              <p className="m-0 text-tc-outside">
                Residual secret warning unavailable.
              </p>
            )}
            <p className="m-0">
              Residual risk: {mutation.data.summary.residual_risk}
            </p>
          </div>
        )}
      </div>
    </ResponsiveOverlay>
  );
}
