import { useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { nearAiNoticeRecovery } from "../health-recovery";
import { useNearAiNoticeRecovery } from "../hooks/use-near-ai-notice-recovery";
import { ButtonPrimary, GlassButton } from "@/design-system";

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
      <div className="text-tc-outside">
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
    <div className="text-tc-outside">
      <strong>{copy.recovery_title}</strong>
      <span>{copy.recovery_detail}</span>
      {since && <small>Since {new Date(since).toLocaleString()}</small>}
      <GlassButton
        type="button"
        className="mt-2 justify-self-start"
        onClick={() => {
          mutation.reset();
          setOpen(true);
        }}
      >
        {copy.recovery_action}
      </GlassButton>
      <ResponsiveOverlay
        open={open}
        onOpenChange={(next) => {
          if (!mutation.isPending) setOpen(next);
        }}
        title={copy.title}
        footer={
          <div className="flex justify-end gap-2">
            <GlassButton
              type="button"
              onClick={() => setOpen(false)}
              disabled={mutation.isPending}
            >
              {copy.recovery_cancel}
            </GlassButton>
            <ButtonPrimary size="sm"
              type="button"
              onClick={() =>
                mutation.mutate(undefined, { onSuccess: () => setOpen(false) })
              }
              disabled={mutation.isPending}
            >
              {mutation.isPending ? copy.recovery_working : copy.recovery_confirm}
            </ButtonPrimary>
          </div>
        }
      >
        <div className="grid gap-3 text-[12px] leading-[1.55] text-[var(--tc-text-primary)]">
          <p className="m-0">{copy.local_always}</p>
          <p className="m-0">{copy.offer}</p>
          <p className="m-0 font-semibold">{copy.disclosure}</p>
          {mutation.isError && (
            <p
              className="tc-card tc-card--quiet m-0 border-tc-outside/30 text-tc-outside"
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
