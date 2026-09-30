import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { HistoryDetail } from "../types";
import { SkillLearningWorkflow } from "./skill-learning-workflow";

export function SkillLearningPanel({
  submissionId,
  detail,
}: {
  submissionId: string;
  detail: HistoryDetail;
}) {
  const core = useCoreStatus();
  if (
    detail.contribution_status !== "accepted" ||
    !detail.human_correction ||
    !submissionId
  ) {
    return null;
  }
  return (
    <SkillLearningWorkflow
      key={`${core.scope}:${submissionId}`}
      submissionId={submissionId}
      detail={detail}
    />
  );
}
