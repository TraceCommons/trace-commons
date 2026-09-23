import { OnboardingConnectStep } from "./onboarding-connect-step";
import { OnboardingConsentStep } from "./onboarding-consent-step";
import { OnboardingDoneStep } from "./onboarding-done-step";
import { OnboardingPrivacyStep } from "./onboarding-privacy-step";
import { OnboardingProjectsStep } from "./onboarding-projects-step";
import { OnboardingRootsStep } from "./onboarding-roots-step";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { OnboardingWelcomeStep } from "./onboarding-welcome-step";

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
    case "privacy":
      return <OnboardingPrivacyStep {...props} />;
    case "projects":
      return <OnboardingProjectsStep {...props} />;
    case "done":
      return <OnboardingDoneStep {...props} />;
  }
}
