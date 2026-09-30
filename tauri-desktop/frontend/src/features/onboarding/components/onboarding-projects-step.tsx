import { Button } from "@/components/ui/button";
import { ProjectsPanel } from "../../settings/public";
import type { OnboardingStepProps } from "./onboarding-step-types";

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
        <Button
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </Button>
        <Button
          className="tc-btn tc-btn--primary tc-btn--sm"
          type="button"
          onClick={onboarding.finishProjects}
          disabled={busy}
        >
          Continue
        </Button>
      </div>
    </>
  );
}
