import { Button } from "@/components/ui/button";
import { scrubDisclosureLines } from "../../../lib/tauri/automatic-grant-copy";
import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { OnboardingStepProps } from "./onboarding-step-types";

// K11, first screen: what is removed before an automatic send, and that no
// one reviews it. The wording is the one the contributor core chose for this
// configuration (R1: "trust relaxes what may be sent, never what may be
// said"); this screen never picks between the patterns-only and model-scrub
// sentences, and has no fallback of its own.
export function OnboardingScrubDisclosureStep({
  onboarding,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "busy">) {
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  const copy = grantCopy.data;
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        WHAT IS REMOVED
      </span>
      <h2>Before a session is sent</h2>
      {copy ? (
        <div className="grid gap-3">
          {scrubDisclosureLines(copy).map((line) => (
            <p className="m-0" key={line}>
              {line}
            </p>
          ))}
        </div>
      ) : (
        <p
          className={`m-0 text-[12px] ${grantCopy.isError ? "text-destructive" : "text-muted-foreground"}`}
          role={grantCopy.isError ? "alert" : "status"}
        >
          {grantCopy.isError
            ? "The disclosure could not be loaded. Continue is disabled."
            : "Loading disclosure…"}
        </p>
      )}
      <div className="mt-3 flex flex-wrap gap-2">
        <Button
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </Button>
        <Button
          className="tc-btn tc-btn--primary tc-btn--sm"
          type="button"
          onClick={onboarding.acknowledgeScrubDisclosure}
          disabled={busy || !copy}
        >
          Continue
        </Button>
      </div>
    </section>
  );
}
