import { OnboardingConnectStep } from "./onboarding-connect-step";
import { OnboardingConsentStep } from "./onboarding-consent-step";
import { OnboardingDoneStep } from "./onboarding-done-step";
import { OnboardingGrantStep } from "./onboarding-grant-step";
import { OnboardingInferenceStep } from "./onboarding-inference-step";
import { OnboardingPathStep } from "./onboarding-path-step";
import { OnboardingProjectsStep } from "./onboarding-projects-step";
import { OnboardingRootsStep } from "./onboarding-roots-step";
import { OnboardingScrubDisclosureStep } from "./onboarding-scrub-disclosure-step";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { OnboardingWelcomeStep } from "./onboarding-welcome-step";
import { OnboardingWitnessDisclosureStep } from "./onboarding-witness-disclosure-step";

export function OnboardingStepContent(props: OnboardingStepProps) {
  switch (props.step) {
    case "welcome":
      return <OnboardingWelcomeStep {...props} />;
    case "roots":
      return <OnboardingRootsStep {...props} />;
    case "connect":
      return <OnboardingConnectStep {...props} />;
    case "consent":
      return <OnboardingConsentStep {...props} />;
    case "path":
      return <OnboardingPathStep {...props} />;
    // The shared Flow 1 model keeps a slot for the retired NEAR AI second
    // scan's step. Nothing enters it any more; if anything did, it goes on
    // to inference, where the step led.
    case "privacy":
    case "inference":
      return <OnboardingInferenceStep {...props} />;
    case "disclosure_scrub":
      return <OnboardingScrubDisclosureStep {...props} />;
    case "disclosure_witness":
      return <OnboardingWitnessDisclosureStep {...props} />;
    case "grant":
      return <OnboardingGrantStep {...props} />;
    case "projects":
      return <OnboardingProjectsStep {...props} />;
    case "done":
      return <OnboardingDoneStep {...props} />;
  }
}
