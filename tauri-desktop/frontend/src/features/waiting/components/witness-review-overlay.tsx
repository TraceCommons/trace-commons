import type { UseMutationResult } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import {
  useRedactionSummary,
  useResidualSecretCopy,
  useWitnessReviewCopy,
} from "../../../lib/tauri/use-contributor-copy";
import type { WitnessReview } from "../api/native-review-api";

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
  const error = mutation.isError
    ? (copy.data?.failed ??
      "Witness review result could not be confirmed. Check the entry's current state before retrying.")
    : mutation.isSuccess && !mutation.data.ready
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
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            {copy.data?.cancel ?? "Cancel"}
          </Button>
          <Button
            type="button"
            onClick={() => mutation.mutate(true)}
            disabled={
              mutation.isPending ||
              mutation.isSuccess ||
              !copy.data ||
              !confirmed
            }
          >
            {mutation.isPending
              ? "Reviewing…"
              : (copy.data?.confirm ?? "Confirm witness review")}
          </Button>
        </div>
      }
    >
      <div className="grid gap-3 text-[12px] text-muted-foreground">
        {copy.isPending && <p className="m-0">Loading review disclosure…</p>}
        {copy.isError && (
          <p className="m-0 text-destructive">
            Disclosure unavailable. Witness review is disabled.
          </p>
        )}
        {copy.data && (
          <label className="flex items-start gap-2.5 rounded-[9px] border border-border p-3.5 text-foreground">
            <Checkbox
              checked={confirmed}
              onCheckedChange={(value) => setConfirmed(value === true)}
              disabled={mutation.isPending || mutation.isSuccess}
              aria-label="Confirm sending unredacted session to witness"
            />
            <span>I understand and want to send this session for review.</span>
          </label>
        )}
        {mutation.isPending && copy.data && (
          <p className="m-0">{copy.data.working}</p>
        )}
        {error && (
          <p className="m-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-destructive">
            {error}
          </p>
        )}
        {mutation.isSuccess && mutation.data.ready && (
          <div className="grid gap-2 text-primary">
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
              <p className="m-0 text-destructive" key={item.family}>
                {item.occurrences} {item.display} remain: {item.description}
              </p>
            ))}
            {redactionSummary.isError && (
              <p className="m-0 text-destructive">
                Redaction details unavailable.
              </p>
            )}
            {residualCopy.data && (
              <p className="m-0 text-destructive">{residualCopy.data}</p>
            )}
            {residualCopy.isError && !residualCopy.data && (
              <p className="m-0 text-destructive">
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
