import { useOnboardingNearAi } from "../hooks/use-onboarding-near-ai";
import { useOnboardingWallet } from "../hooks/use-onboarding-wallet";
import { InviteConnectForm } from "./invite-connect-form";
import { OnboardingNearAiJoin } from "./onboarding-near-ai-join";
import type { OnboardingController } from "./onboarding-step-types";
import { OnboardingWalletConnect } from "./onboarding-wallet-connect";
import { GlassButton } from "@/design-system";

export function OnboardingConnectionOptions({
  onboarding,
  initialInvite,
  busy,
}: {
  onboarding: OnboardingController;
  initialInvite?: string | null;
  busy: boolean;
}) {
  const onEnrolled = () => void onboarding.markEnrolled();
  const nearAi = useOnboardingNearAi(onEnrolled);
  const wallet = useOnboardingWallet(onEnrolled);
  const nativeBusy = nearAi.busy || wallet.pending;

  return (
    <>
      <InviteConnectForm
        busy={busy || nativeBusy}
        initialInvite={initialInvite}
        onConnect={onboarding.enroll}
      />
      <OnboardingNearAiJoin nearAi={nearAi} blocked={busy || wallet.pending} />
      <OnboardingWalletConnect wallet={wallet} blocked={busy || nearAi.busy} />
      <div className="flex gap-2.5 border-t border-tc-hairline pt-5">
        <GlassButton
          type="button"
          onClick={onboarding.back}
          disabled={busy || nativeBusy}
        >
          Back
        </GlassButton>
      </div>
    </>
  );
}
