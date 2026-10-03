import { ProjectsPanel } from "../../settings/public";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { ButtonPrimary, GlassButton } from "@/design-system";

export function OnboardingProjectsStep({
  onboarding,
  projects,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "projects" | "busy">) {
  return (
    <>
      <ProjectsPanel
        projects={projects.projects}
        state={projects.state}
        error={projects.error}
        onRefresh={projects.refresh}
        onSetMode={projects.setMode}
        allowAutoUpload={false}
      />
      <div className="mt-3 flex flex-wrap gap-2">
        <GlassButton
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </GlassButton>
        <ButtonPrimary size="sm"
          type="button"
          onClick={onboarding.finishProjects}
          disabled={busy}
        >
          Continue
        </ButtonPrimary>
      </div>
    </>
  );
}
