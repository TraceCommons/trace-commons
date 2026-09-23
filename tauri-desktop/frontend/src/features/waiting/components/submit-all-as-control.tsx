import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { OutcomeVerdict } from "../types";

export function SubmitAllAsControl({
  eligibleCount,
  busy,
  onSubmit,
}: {
  eligibleCount: number;
  busy: boolean;
  onSubmit: (outcome: OutcomeVerdict) => void;
}) {
  const [open, setOpen] = useState(false);
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data?.outcome;
  if (!copy) return null;
  const submit = (outcome: OutcomeVerdict) => {
    setOpen(false);
    onSubmit(outcome);
  };
  return (
    <>
      <Button
        type="button"
        variant="outline"
        title={copy.submit_all_as_tooltip}
        onClick={() => setOpen(true)}
        disabled={busy || eligibleCount === 0}
      >
        {copy.submit_all_as}
      </Button>
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title={copy.submit_all_as}
        description={copy.submit_all_as_tooltip}
        footer={
          <Button
            type="button"
            variant="outline"
            onClick={() => setOpen(false)}
            disabled={busy}
          >
            Cancel
          </Button>
        }
      >
        <div className="grid gap-2.5">
          <p className="m-0 text-sm text-muted-foreground">
            Apply one outcome to {eligibleCount} eligible session
            {eligibleCount === 1 ? "" : "s"}.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button type="button" onClick={() => submit("worked")} disabled={busy}>
              {copy.worked}
            </Button>
            <Button type="button" variant="outline" onClick={() => submit("partly")} disabled={busy}>
              {copy.partly}
            </Button>
            <Button type="button" variant="outline" onClick={() => submit("failed")} disabled={busy}>
              {copy.failed}
            </Button>
          </div>
        </div>
      </ResponsiveOverlay>
    </>
  );
}
