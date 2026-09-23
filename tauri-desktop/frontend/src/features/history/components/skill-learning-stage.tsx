import { Button } from "@/components/ui/button";
import { useExternalUrl } from "../../../lib/tauri/use-platform-actions";
import type { useSkillLearning } from "../hooks/use-skill-learning";
import { SkillCandidateForm } from "./skill-candidate-form";
import { SkillEvaluationResults } from "./skill-evaluation-results";
import { SkillInstallPreview } from "./skill-install-preview";
import { SkillReviewPreview } from "./skill-review-preview";

type SkillLearning = ReturnType<typeof useSkillLearning>;

export function SkillLearningStage({ skill }: { skill: SkillLearning }) {
  const externalUrl = useExternalUrl();
  const busy = skill.busy || skill.statusUnavailable || externalUrl.isPending;
  switch (skill.stage) {
    case "idle":
      return (
        <div className="mt-6 flex gap-2.5">
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="button"
            onClick={() => void skill.learn()}
            disabled={busy}
          >
            Learn from session
          </Button>
        </div>
      );
    case "candidate":
      return skill.candidate && skill.copy ? (
        <SkillCandidateForm
          candidate={skill.candidate}
          copy={skill.copy}
          busy={busy}
          onReview={(draft) => void skill.reviewCandidate(draft)}
        />
      ) : null;
    case "reviewed":
      return skill.review && skill.copy ? (
        <SkillReviewPreview
          copy={skill.copy}
          review={skill.review}
          busy={busy}
          onEdit={skill.editReview}
          onApprove={() => void skill.evaluate()}
        />
      ) : null;
    case "evaluated":
      return skill.report && skill.copy ? (
        <SkillEvaluationResults
          copy={skill.copy}
          report={skill.report}
          busy={busy}
          onReviewInstall={() => void skill.prepareInstall()}
          onInspect={(url) => void externalUrl.open(url)}
        />
      ) : null;
    case "planned":
    case "installed":
      return skill.copy && (skill.plan || skill.installed) ? (
        <SkillInstallPreview
          copy={skill.copy}
          plan={skill.plan}
          installed={skill.installed}
          busy={busy}
          onInstall={() => void skill.install()}
          onRollback={() => void skill.rollback()}
        />
      ) : null;
    default:
      return null;
  }
}
