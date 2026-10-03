import { OnboardingConnectionOptions } from "./onboarding-connection-options";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { ButtonPrimary } from "@/design-system";

export function OnboardingConnectStep({
  onboarding,
  alreadyEnrolled,
  initialInvite,
  busy,
}: Pick<
  OnboardingStepProps,
  "onboarding" | "alreadyEnrolled" | "initialInvite" | "busy"
>) {
  return (
    <section className="mb-4 grid gap-5 tc-card">
      <div>
        <span className="mb-1.5 block tc-eyebrow">
          CONNECT
        </span>
        <h2>Connect a commons</h2>
        <p className="m-0 tc-label font-normal leading-[17px] tc-text-secondary">
          Choose an invite, existing NEAR AI sign-in, or NEAR wallet. Enrollment
          happens only after you confirm one path.
        </p>
      </div>
      {alreadyEnrolled ? (
        <div className="grid gap-3">
          <p className="m-0 tc-label font-normal tc-text-secondary">
            This device is already connected.
          </p>
          <ButtonPrimary size="sm" type="button" onClick={() => void onboarding.markEnrolled()}>
            Continue
          </ButtonPrimary>
        </div>
      ) : (
        <OnboardingConnectionOptions
          onboarding={onboarding}
          initialInvite={initialInvite}
          busy={busy}
        />
      )}
    </section>
  );
}
