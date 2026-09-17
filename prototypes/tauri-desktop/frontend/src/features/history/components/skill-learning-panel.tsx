import { useSkillLearning } from "../hooks/use-skill-learning";
import type { HistoryDetail } from "../types";
import { SkillLearningStage } from "./skill-learning-stage";

function stageTitle(stage: ReturnType<typeof useSkillLearning>["stage"]) {
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

export function SkillLearningPanel({
  submissionId,
  detail,
}: {
  submissionId: string;
  detail: HistoryDetail;
}) {
  const skill = useSkillLearning(submissionId, detail);
  if (!skill.eligible) return null;
  return (
    <section className="mt-4 grid gap-[18px] rounded-2xl border border-secondary bg-secondary/50/[.84] p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            SKILL LEARNING
          </span>
          <h2>{stageTitle(skill.stage)}</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          {skill.stage}
        </span>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        {skill.copy?.promise ??
          "Turn this accepted correction into a reviewed, evaluated, owner-controlled Agent Skill."}
      </p>
      {skill.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {skill.error}
        </p>
      )}
      <SkillLearningStage skill={skill} />
    </section>
  );
}
