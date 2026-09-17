import type { useProjects } from "../../settings/hooks/use-projects";
import type { useSettings } from "../../settings/hooks/use-settings";
import type { useSourceRoots } from "../../settings/hooks/use-source-roots";
import type { OnboardingStep, useOnboarding } from "../hooks/use-onboarding";

export type OnboardingController = ReturnType<typeof useOnboarding>;
export type SettingsController = ReturnType<typeof useSettings>;
export type ProjectsController = ReturnType<typeof useProjects>;
export type RootsController = ReturnType<typeof useSourceRoots>;

export type OnboardingStepProps = {
  step: OnboardingStep;
  onboarding: OnboardingController;
  settings: SettingsController;
  projects: ProjectsController;
  roots: RootsController;
  alreadyEnrolled: boolean;
  initialInvite?: string | null;
  busy: boolean;
  showPrivacy: boolean;
  onOpenScrubDisclosure: () => void;
  onComplete: () => void;
};
