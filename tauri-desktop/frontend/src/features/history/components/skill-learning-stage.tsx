import { useExternalUrl } from "../../../lib/tauri/use-platform-actions";
import type { useSkillLearning } from "../hooks/use-skill-learning";
import { SkillCandidateForm } from "./skill-candidate-form";
import { SkillEvaluationResults } from "./skill-evaluation-results";
import { SkillInstallPreview } from "./skill-install-preview";
import { SkillReviewPreview } from "./skill-review-preview";
import { ButtonPrimary } from "@/design-system";

type SkillLearning = ReturnType<typeof useSkillLearning>;

export function SkillLearningStage({ skill }: { skill: SkillLearning }) {
  const externalUrl = useExternalUrl();
  const busy = skill.busy || skill.statusUnavailable || externalUrl.isPending;
  switch (skill.stage) {
    case "idle":
      return (
        <div className="mt-3 flex flex-wrap gap-2">
          <ButtonPrimary size="sm"
            type="button"
            onClick={() => void skill.learn()}
            disabled={busy}
          >
            Learn from session
          </ButtonPrimary>
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
