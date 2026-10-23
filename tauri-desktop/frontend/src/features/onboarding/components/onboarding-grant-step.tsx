import {
  useAutomaticGrantCopy,
  useContributorDisclosureCopy,
} from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { GlassButton } from "@/design-system";

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
  // The two buttons name the folder modes they lead to, by their one name
  // (`folder_mode_labels`); the grant is not offered before those arrive.
  const disclosure = useContributorDisclosureCopy();
  const modes = disclosure.data?.folder_mode_labels;
  const copy = modes ? grantCopy.data : undefined;
  const copyFailed = grantCopy.isError || disclosure.isError;
  const blocked = onboarding.grantBlockers.length > 0;
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        AUTOMATIC CONTRIBUTING
      </span>
      <h2>Turn on automatic contributing?</h2>
      {copy ? (
        <div className="grid gap-3">
          <p className="m-0">{copy.path_automatic}</p>
          <p className="m-0 font-bold">{copy.no_review}</p>
        </div>
      ) : (
        <p
          className={`m-0 text-[12px] ${copyFailed ? "text-tc-outside" : "text-tc-secondary"}`}
          role={copyFailed ? "alert" : "status"}
        >
          {copyFailed
            ? "The disclosure could not be loaded. The grant is disabled."
            : "Loading disclosure…"}
        </p>
      )}
      <div className="mt-6 flex flex-wrap gap-2.5">
        <GlassButton
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </GlassButton>
        <GlassButton
          type="button"
          onClick={onboarding.skipGrant}
          disabled={busy || !modes}
        >
          {modes?.notify_only}
        </GlassButton>
        <GlassButton
          type="button"
          onClick={() => void onboarding.grant()}
          disabled={busy || !copy || blocked}
        >
          {onboarding.grantMutation.isPending
            ? "Turning on…"
            : modes?.auto_upload}
        </GlassButton>
      </div>
    </section>
  );
}
