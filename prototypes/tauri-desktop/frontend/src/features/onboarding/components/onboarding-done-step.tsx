import { Button } from "@/components/ui/button";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingDoneStep({
  onComplete,
}: Pick<OnboardingStepProps, "onComplete">) {
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        DONE
      </span>
      <h2>You're set up. Nothing has been sent.</h2>
      <p>
        Finished sessions will appear in Waiting. You decide each submission.
        Source roots, watcher activation, notifications, and private inference
        remain separate settings.
      </p>
      <Button
        className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
        type="button"
        onClick={onComplete}
      >
        Done
      </Button>
    </section>
  );
}
