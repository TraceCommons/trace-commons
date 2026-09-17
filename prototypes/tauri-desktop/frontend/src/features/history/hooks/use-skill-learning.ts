import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { historyKeys } from "../api/query-keys";
import {
  commitSkillInstall,
  evaluateSkill,
  getSkillCandidate,
  getSkillCopy,
  getSkillInstallStatus,
  planSkillInstall,
  reviewSkill,
  rollbackSkill,
} from "../api/skill-api";
import type {
  InstalledSkill,
  SkillCandidate,
  SkillCopy,
  SkillDraft,
  SkillEvaluationReport,
  SkillInstallPlan,
  SkillReview,
} from "../skill-types";
import type { HistoryDetail } from "../types";

export type SkillStage =
  | "idle"
  | "candidate"
  | "reviewed"
  | "evaluated"
  | "planned"
  | "installed";

export function useSkillLearning(
  sourceSubmissionId: string | null,
  detail: HistoryDetail | null,
) {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [copy, setCopy] = useState<SkillCopy | null>(null);
  const [candidate, setCandidate] = useState<SkillCandidate | null>(null);
  const [review, setReview] = useState<SkillReview | null>(null);
  const [report, setReport] = useState<SkillEvaluationReport | null>(null);
  const [plan, setPlan] = useState<SkillInstallPlan | null>(null);
  const [installed, setInstalled] = useState<InstalledSkill | null>(null);
  const [stage, setStage] = useState<SkillStage>("idle");
  const [error, setError] = useState<string | null>(null);

  const eligible = Boolean(
    detail?.contribution_status === "accepted" &&
      detail.human_correction &&
      sourceSubmissionId,
  );

  const installStatusQuery = useQuery({
    queryKey: historyKeys.skillInstallStatus(
      core.scope,
      sourceSubmissionId ?? "none",
    ),
    queryFn: () => getSkillInstallStatus(sourceSubmissionId as string),
    enabled: core.isSuccess && eligible && sourceSubmissionId !== null,
  });
  const learnMutation = useMutation({
    mutationFn: async () => {
      if (!sourceSubmissionId || !eligible) throw new Error("skill-ineligible");
      const [nextCandidate, nextCopy] = await Promise.all([
        queryClient.fetchQuery({
          queryKey: historyKeys.skillCandidate(core.scope, sourceSubmissionId),
          queryFn: () => getSkillCandidate(sourceSubmissionId),
        }),
        queryClient.fetchQuery({
          queryKey: historyKeys.skillCopy(),
          queryFn: getSkillCopy,
        }),
      ]);
      return { nextCandidate, nextCopy };
    },
    onSuccess: ({ nextCandidate, nextCopy }) => {
      setCandidate(nextCandidate);
      setCopy(nextCopy);
      setStage("candidate");
    },
  });
  const reviewMutation = useMutation({
    mutationFn: ({
      candidateId,
      nextDraft,
      replacesReviewId,
    }: {
      candidateId: string;
      nextDraft: SkillDraft;
      replacesReviewId: string | null;
    }) => reviewSkill(candidateId, nextDraft, replacesReviewId),
    onSuccess: (nextReview) => {
      setReview(nextReview);
      setStage("reviewed");
    },
  });
  const evaluateMutation = useMutation({
    mutationFn: ({
      reviewId,
      skillSha256,
    }: {
      reviewId: string;
      skillSha256: string;
    }) => evaluateSkill(reviewId, skillSha256),
    onSuccess: (nextReport) => {
      setReport(nextReport);
      setStage("evaluated");
    },
  });
  const planMutation = useMutation({
    mutationFn: (evaluationId: string) => planSkillInstall(evaluationId),
    onSuccess: (nextPlan) => {
      setPlan(nextPlan);
      setStage("planned");
    },
  });
  const installMutation = useMutation({
    mutationFn: (nextPlan: SkillInstallPlan) => commitSkillInstall(nextPlan),
    onSuccess: async (nextInstalled) => {
      setInstalled(nextInstalled);
      setStage("installed");
      if (sourceSubmissionId) {
        await queryClient.invalidateQueries({
          queryKey: historyKeys.scope(core.scope),
        });
      }
    },
  });
  const rollbackMutation = useMutation({
    mutationFn: (nextInstalled: InstalledSkill) => rollbackSkill(nextInstalled),
    onSuccess: async (result) => {
      if (!result.removed) throw new Error("rollback-incomplete");
      setInstalled(null);
      setPlan(null);
      setStage(report ? "evaluated" : "idle");
      if (sourceSubmissionId) {
        await queryClient.invalidateQueries({
          queryKey: historyKeys.scope(core.scope),
        });
      }
    },
  });

  useEffect(() => {
    setCandidate(null);
    setReview(null);
    setReport(null);
    setPlan(null);
    setInstalled(null);
    setStage("idle");
    setError(null);
    if (eligible && installStatusQuery.data) {
      setInstalled(installStatusQuery.data);
      setStage("installed");
    }
  }, [eligible, installStatusQuery.data]);

  const busy =
    learnMutation.isPending ||
    reviewMutation.isPending ||
    evaluateMutation.isPending ||
    planMutation.isPending ||
    installMutation.isPending ||
    rollbackMutation.isPending;
  const workflowError =
    error ??
    (learnMutation.isError ||
    reviewMutation.isError ||
    evaluateMutation.isError ||
    planMutation.isError ||
    installMutation.isError ||
    rollbackMutation.isError
      ? "Skill workflow could not complete. Retry this step."
      : null);

  const learn = async () => {
    setError(null);
    try {
      await learnMutation.mutateAsync();
    } catch {
      // Query/mutation state supplies the workflow error.
    }
  };

  const reviewCandidate = async (draft: SkillDraft) => {
    if (!candidate) return;
    try {
      await reviewMutation.mutateAsync({
        candidateId: candidate.candidate_id,
        nextDraft: draft,
        replacesReviewId: candidate.replaces_review_id,
      });
    } catch {
      // Query/mutation state supplies the workflow error.
    }
  };

  const editReview = () => {
    if (!candidate || !review) return;
    setCandidate({
      ...candidate,
      draft: review.draft,
      replaces_review_id: review.review_id,
    });
    setReview(null);
    setReport(null);
    setPlan(null);
    setStage("candidate");
    setError(null);
  };

  const evaluate = async () => {
    if (!review) return;
    try {
      await evaluateMutation.mutateAsync({
        reviewId: review.review_id,
        skillSha256: review.skill_sha256,
      });
    } catch {
      // Query/mutation state supplies the workflow error.
    }
  };

  const prepareInstall = async () => {
    if (!report?.install_allowed) return;
    try {
      await planMutation.mutateAsync(report.evaluation_id);
    } catch {
      // Query/mutation state supplies the workflow error.
    }
  };

  const install = async () => {
    if (!plan) return;
    try {
      await installMutation.mutateAsync(plan);
    } catch {
      // Query/mutation state supplies the workflow error.
    }
  };

  const rollback = async () => {
    if (!installed) return;
    try {
      await rollbackMutation.mutateAsync(installed);
    } catch {
      // Query/mutation state supplies the workflow error.
    }
  };

  const reset = () => {
    setCandidate(null);
    setReview(null);
    setReport(null);
    setPlan(null);
    setStage("idle");
    setError(null);
  };

  return {
    ...installStatusQuery,
    eligible,
    copy,
    candidate,
    review,
    report,
    plan,
    installed,
    stage,
    busy,
    error: workflowError,
    learn,
    reviewCandidate,
    editReview,
    evaluate,
    prepareInstall,
    install,
    rollback,
    reset,
    installStatusQuery,
    learnMutation,
    reviewMutation,
    evaluateMutation,
    planMutation,
    installMutation,
    rollbackMutation,
  };
}
