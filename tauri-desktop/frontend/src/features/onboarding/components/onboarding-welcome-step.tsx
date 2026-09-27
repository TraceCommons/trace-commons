import { Button } from "@/components/ui/button";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingWelcomeStep({
  onboarding,
  onOpenScrubDisclosure,
}: Pick<OnboardingStepProps, "onboarding" | "onOpenScrubDisclosure">) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px] max-w-[760px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        TRACE COMMONS
      </span>
      <h2>{copy?.onboarding.heading ?? "Your first contribution"}</h2>
      {copy ? (
        <>
          <p>{copy.onboarding_shell.welcome_body}</p>
          <p>{copy.onboarding.start}</p>
        </>
      ) : (
        <p role="alert">Shared onboarding copy unavailable. Retry loading it.</p>
      )}
      <Button
        className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
        type="button"
        onClick={onOpenScrubDisclosure}
      >
        What gets removed?
      </Button>
      <p className="mt-2 font-bold text-foreground">
        You decide what gets contributed. Nothing is sent unless you say so.
      </p>
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onboarding.startRoots}
          disabled={!copy}
        >
          Get started
        </Button>
      </div>
    </section>
  );
}
