import { useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { OutcomeVerdict } from "../types";
import { ButtonPrimary, GlassButton } from "@/design-system";

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
      <GlassButton
        type="button"
        title={copy.submit_all_as_tooltip}
        onClick={() => setOpen(true)}
        disabled={busy || eligibleCount === 0}
      >
        {copy.submit_all_as}
      </GlassButton>
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title={copy.submit_all_as}
        description={copy.submit_all_as_tooltip}
        footer={
          <GlassButton
            type="button"
            onClick={() => setOpen(false)}
            disabled={busy}
          >
            Cancel
          </GlassButton>
        }
      >
        <div className="grid gap-2.5">
          <p className="m-0 text-sm text-tc-secondary">
            Apply one outcome to {eligibleCount} eligible session
            {eligibleCount === 1 ? "" : "s"}.
          </p>
          <div className="flex flex-wrap gap-2">
            <ButtonPrimary size="sm" type="button" onClick={() => submit("worked")} disabled={busy}>
              {copy.worked}
            </ButtonPrimary>
            <GlassButton type="button" onClick={() => submit("partly")} disabled={busy}>
              {copy.partly}
            </GlassButton>
            <GlassButton type="button" onClick={() => submit("failed")} disabled={busy}>
              {copy.failed}
            </GlassButton>
          </div>
        </div>
      </ResponsiveOverlay>
    </>
  );
}
