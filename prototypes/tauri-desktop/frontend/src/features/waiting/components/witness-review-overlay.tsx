import type { UseMutationResult } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import type { WitnessReview } from "../api/native-review-api";

type WitnessMutation = UseMutationResult<WitnessReview, Error, void, unknown>;

export function WitnessReviewOverlay({
  open,
  onOpenChange,
  mutation,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mutation: WitnessMutation;
}) {
  const error = mutation.isError
    ? "Witness review could not be completed. Nothing was sent."
    : mutation.isSuccess && !mutation.data.ready
      ? mutation.data.message
      : null;
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title="Request witness review"
      description="This sends an explicit raw-session review request to the pinned witness. It does not approve or publish the entry."
      footer={
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            onClick={() => mutation.mutate()}
            disabled={mutation.isPending}
          >
            {mutation.isPending ? "Reviewing…" : "Confirm witness review"}
          </Button>
        </div>
      }
    >
      <div className="grid gap-3 text-[12px] text-muted-foreground">
        <p className="m-0">
          The witness receives only this explicit review request. Ordinary
          preview and approval paths do not call it.
        </p>
        {error && (
          <p className="m-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-destructive">
            {error}
          </p>
        )}
        {mutation.isSuccess && mutation.data.ready && (
          <p className="m-0 text-primary">
            {mutation.data.summary ?? "Witness review is ready."}
          </p>
        )}
      </div>
    </ResponsiveOverlay>
  );
}
