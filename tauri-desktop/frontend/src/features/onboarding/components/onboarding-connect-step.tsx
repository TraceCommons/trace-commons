import { Button } from "@/components/ui/button";
import { OnboardingConnectionOptions } from "./onboarding-connection-options";
import type { OnboardingStepProps } from "./onboarding-step-types";

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
    <section className="mb-4 grid gap-5 rounded-2xl border border-border bg-card/80 p-[26px]">
      <div>
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          CONNECT
        </span>
        <h2>Connect a commons</h2>
        <p className="m-0 text-[12px] leading-[1.55] text-muted-foreground">
          Choose an invite, existing NEAR AI sign-in, or NEAR wallet. Enrollment
          happens only after you confirm one path.
        </p>
      </div>
      {alreadyEnrolled ? (
        <div className="grid gap-3">
          <p className="m-0 text-[12px] text-muted-foreground">
            This device is already connected.
          </p>
          <Button type="button" onClick={() => void onboarding.markEnrolled()}>
            Continue
          </Button>
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
