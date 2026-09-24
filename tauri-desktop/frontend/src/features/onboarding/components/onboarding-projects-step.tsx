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
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
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
