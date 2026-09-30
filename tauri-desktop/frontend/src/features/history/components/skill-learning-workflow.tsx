import { Button } from "@/components/ui/button";
import { useSkillLearning } from "../hooks/use-skill-learning";
import type { HistoryDetail } from "../types";
import { SkillLearningStage } from "./skill-learning-stage";

function stageTitle(
  stage: ReturnType<typeof useSkillLearning>["stage"],
): string {
  switch (stage) {
    case "candidate":
      return "Candidate skill";
    case "reviewed":
      return "Review skill";
    case "evaluated":
      return "Skill evaluation";
    case "planned":
    case "installed":
      return "Install skill";
    default:
      return "Learn from session";
  }
}

export function SkillLearningWorkflow({
  submissionId,
  detail,
}: {
  submissionId: string;
  detail: HistoryDetail;
}) {
  const skill = useSkillLearning(submissionId, detail);
  return (
    <section className="tc-card mt-4 grid gap-[18px]">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            SKILL LEARNING
          </span>
          <h2>{stageTitle(skill.stage)}</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
          {skill.stage}
        </span>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        {skill.copy?.promise ??
          "Turn this accepted correction into a reviewed, evaluated, owner-controlled Agent Skill."}
      </p>
      {skill.error && (
        <p className="tc-alert">
          {skill.error}
        </p>
      )}
      {skill.statusUnavailable && (
        <Button
          className="w-fit"
          type="button"
          variant="outline"
          onClick={() => void skill.retryInstallStatus()}
        >
          Retry installed skill status
        </Button>
      )}
      <SkillLearningStage skill={skill} />
    </section>
  );
}
