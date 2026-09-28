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
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
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
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
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
