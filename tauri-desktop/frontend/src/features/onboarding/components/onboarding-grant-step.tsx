import { Button } from "@/components/ui/button";
import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { OnboardingStepProps } from "./onboarding-step-types";

// The Flow 1 grant. Reachable only after connect, a chosen scope, the
// automatic path and both disclosure screens; the button stays off while any
// of them is missing, and `requestGrant` refuses again underneath it. As on
// the arming offer, the non-consequential action comes first and the grant
// is not visually accented.
export function OnboardingGrantStep({
  onboarding,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "busy">) {
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  const copy = grantCopy.data;
  const blocked = onboarding.grantBlockers.length > 0;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        AUTOMATIC CONTRIBUTING
      </span>
      <h2>Turn on automatic contributing?</h2>
      {copy ? (
        <div className="grid gap-3">
          <p className="m-0">{copy.path_automatic}</p>
          <p className="m-0 font-bold">{copy.no_review}</p>
        </div>
      ) : (
        <p className="m-0 text-[12px] text-destructive" role="alert">
          {grantCopy.isError
            ? "The disclosure could not be loaded. The grant is disabled."
            : "Loading disclosure…"}
        </p>
      )}
      <div className="mt-6 flex flex-wrap gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={onboarding.skipGrant}
          disabled={busy}
        >
          Ask me each time instead
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => void onboarding.grant()}
          disabled={busy || !copy || blocked}
        >
          {onboarding.grantMutation.isPending
            ? "Turning on…"
            : "Contribute automatically"}
        </Button>
      </div>
    </section>
  );
}
