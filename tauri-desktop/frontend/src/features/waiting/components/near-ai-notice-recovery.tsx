import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { nearAiNoticeRecovery } from "../health-recovery";
import { useNearAiNoticeRecovery } from "../hooks/use-near-ai-notice-recovery";

// The daemon holds every upload while `near-ai-notice-not-acknowledged` is
// reported. Onboarding is not the only place that can happen, so the queue
// offers the same shared notice and the same acknowledgement.
export function NearAiNoticeRecovery({
  label,
  since,
}: {
  label: string;
  since: string | null;
}) {
  const disclosure = useContributorDisclosureCopy();
  const mutation = useNearAiNoticeRecovery();
  const [open, setOpen] = useState(false);
  const recovery = nearAiNoticeRecovery(
    label,
    disclosure.data?.privacy_scan,
    disclosure.isError,
  );
  if (recovery.kind === "none") return null;
  if (recovery.kind !== "ready") {
    return (
      <div className="text-destructive">
        <span role={recovery.kind === "unavailable" ? "alert" : undefined}>
          {recovery.kind === "unavailable"
            ? "Privacy scan notice unavailable. Confirmation is disabled."
            : "Loading privacy scan notice…"}
        </span>
      </div>
    );
  }
  const copy = recovery.copy;
  return (
    <div className="text-destructive">
      <strong>{copy.recovery_title}</strong>
      <span>{copy.recovery_detail}</span>
      {since && <small>Since {new Date(since).toLocaleString()}</small>}
      <Button
        type="button"
        size="sm"
        variant="outline"
        className="mt-2 justify-self-start"
        onClick={() => {
          mutation.reset();
          setOpen(true);
        }}
      >
        {copy.recovery_action}
      </Button>
      <ResponsiveOverlay
        open={open}
        onOpenChange={(next) => {
          if (!mutation.isPending) setOpen(next);
        }}
        title={copy.title}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setOpen(false)}
              disabled={mutation.isPending}
            >
              {copy.recovery_cancel}
            </Button>
            <Button
              type="button"
              onClick={() =>
                mutation.mutate(undefined, { onSuccess: () => setOpen(false) })
              }
              disabled={mutation.isPending}
            >
              {mutation.isPending ? copy.recovery_working : copy.recovery_confirm}
            </Button>
          </div>
        }
      >
        <div className="grid gap-3 text-[12px] leading-[1.55] text-foreground">
          <p className="m-0">{copy.local_always}</p>
          <p className="m-0">{copy.offer}</p>
          <p className="m-0 font-semibold">{copy.disclosure}</p>
          {mutation.isError && (
            <p
              className="m-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-destructive"
              role="alert"
            >
              {copy.recovery_failed}
            </p>
          )}
        </div>
      </ResponsiveOverlay>
    </div>
  );
}
