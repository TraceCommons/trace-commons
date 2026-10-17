import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { ButtonPrimary, TertiaryLink } from "@/design-system";

export function OnboardingWelcomeStep({
  onboarding,
  onOpenScrubDisclosure,
}: Pick<OnboardingStepProps, "onboarding" | "onOpenScrubDisclosure">) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data;
  return (
    <section className="tc-card mb-2.5 max-w-[760px]">
      <span className="mb-1.5 block tc-eyebrow">
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
      <TertiaryLink
        type="button"
        onClick={onOpenScrubDisclosure}
      >
        What gets removed?
      </TertiaryLink>
      <p className="mt-2 font-bold text-[var(--tc-text-primary)]">
        You decide what gets contributed. Nothing is sent unless you say so.
      </p>
      <div className="mt-3 flex flex-wrap gap-2">
        <ButtonPrimary size="sm"
          type="button"
          onClick={onboarding.startRoots}
          disabled={!copy}
        >
          Get started
        </ButtonPrimary>
      </div>
    </section>
  );
}
