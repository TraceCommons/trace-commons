import { invokeTauri } from "../../../lib/tauri/core-api";
import type {
  InstalledSkill,
  SkillCandidate,
  SkillCopy,
  SkillDraft,
  SkillEvaluationContract,
  SkillEvaluationReport,
  SkillInstallPlan,
  SkillReview,
  SkillSourceEvidence,
  SkillTrial,
} from "../skill-types";

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(`Invalid skill ${label}`);
  return value as Record<string, unknown>;
}
function stringField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid skill field: ${key}`);
  return value[key] as string;
}
function numberField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "number" || !Number.isInteger(value[key]))
    throw new Error(`Invalid skill field: ${key}`);
  return value[key] as number;
}
function booleanField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "boolean")
    throw new Error(`Invalid skill field: ${key}`);
  return value[key] as boolean;
}
function strings(value: Record<string, unknown>, key: string) {
  if (
    !Array.isArray(value[key]) ||
    !value[key].every((item) => typeof item === "string")
  )
    throw new Error(`Invalid skill list: ${key}`);
  return value[key] as string[];
}

function draft(value: unknown): SkillDraft {
  const item = record(value, "draft");
  return {
    name: stringField(item, "name"),
    description: stringField(item, "description"),
    procedure: stringField(item, "procedure"),
  };
}
function contract(value: unknown): SkillEvaluationContract {
  const item = record(value, "contract");
  return {
    task_count: numberField(item, "task_count"),
    plan_task_count: numberField(item, "plan_task_count"),
    fixture_pool_count: numberField(item, "fixture_pool_count"),
    cluster_count: numberField(item, "cluster_count"),
    arm_count: numberField(item, "arm_count"),
    total_requests: numberField(item, "total_requests"),
    output_token_limit: numberField(item, "output_token_limit"),
    request_timeout_seconds: numberField(item, "request_timeout_seconds"),
    required_model_owner: stringField(item, "required_model_owner"),
    fixture_scope: stringField(item, "fixture_scope"),
    selection_policy: stringField(item, "selection_policy"),
  };
}
function evidence(value: unknown): SkillSourceEvidence {
  const item = record(value, "evidence");
  return {
    event_id: stringField(item, "event_id"),
    kind: stringField(item, "kind"),
    excerpt: stringField(item, "excerpt"),
  };
}
function candidate(value: unknown): SkillCandidate {
  const item = record(value, "candidate");
  return {
    candidate_id: stringField(item, "candidate_id"),
    family: stringField(item, "family"),
    source_submission_id: stringField(item, "source_submission_id"),
    source_correction: stringField(item, "source_correction"),
    source_evidence: Array.isArray(item.source_evidence)
      ? item.source_evidence.map(evidence)
      : [],
    draft: draft(item.draft),
    replaces_review_id:
      item.replaces_review_id === null || item.replaces_review_id === undefined
        ? null
        : stringField(item, "replaces_review_id"),
    manual_control_instruction: stringField(item, "manual_control_instruction"),
    evaluation_contract: contract(item.evaluation_contract),
  };
}
function review(value: unknown): SkillReview {
  const item = record(value, "review");
  return {
    review_id: stringField(item, "review_id"),
    candidate_id: stringField(item, "candidate_id"),
    source_submission_id: stringField(item, "source_submission_id"),
    source_evidence_ids: strings(item, "source_evidence_ids"),
    draft: draft(item.draft),
    skill_md: stringField(item, "skill_md"),
    skill_sha256: stringField(item, "skill_sha256"),
  };
}
function armSummary(value: unknown) {
  const item = record(value, "summary");
  return {
    arm: stringField(item, "arm"),
    label: stringField(item, "label"),
    passed: numberField(item, "passed"),
    total: numberField(item, "total"),
  };
}
function optionalString(value: Record<string, unknown>, key: string) {
  return value[key] === null || value[key] === undefined
    ? null
    : stringField(value, key);
}
function optionalNumber(value: Record<string, unknown>, key: string) {
  return value[key] === null || value[key] === undefined
    ? null
    : numberField(value, key);
}
function usage(value: unknown) {
  const item = record(value, "usage");
  return {
    prompt_tokens: optionalNumber(item, "prompt_tokens"),
    completion_tokens: optionalNumber(item, "completion_tokens"),
    reasoning_tokens: optionalNumber(item, "reasoning_tokens"),
    total_tokens: optionalNumber(item, "total_tokens"),
  };
}
function trial(value: unknown): SkillTrial {
  const item = record(value, "trial");
  const answer =
    item.answer === null || item.answer === undefined
      ? null
      : (() => {
          const result = record(item.answer, "answer");
          return {
            diagnosis: stringField(result, "diagnosis"),
            edit_paths: strings(result, "edit_paths"),
            commands: strings(result, "commands"),
            verification: strings(result, "verification"),
          };
        })();
  return {
    task_id: stringField(item, "task_id"),
    cluster: stringField(item, "cluster"),
    task: stringField(item, "task"),
    source_url: stringField(item, "source_url"),
    arm: stringField(item, "arm"),
    passed: booleanField(item, "passed"),
    failure_reasons: strings(item, "failure_reasons"),
    answer,
    raw_output: stringField(item, "raw_output"),
    request_id: optionalString(item, "request_id"),
    served_model: stringField(item, "served_model"),
    finish_reason: stringField(item, "finish_reason"),
    usage: usage(item.usage),
  };
}
function report(value: unknown): SkillEvaluationReport {
  const item = record(value, "evaluation");
  return {
    evaluation_id: stringField(item, "evaluation_id"),
    review_id: stringField(item, "review_id"),
    skill_sha256: stringField(item, "skill_sha256"),
    evaluation_contract: contract(item),
    requested_model: stringField(item, "requested_model"),
    served_model: stringField(item, "served_model"),
    model_owned_by: stringField(item, "model_owned_by"),
    source_task_fingerprint_sha256: stringField(
      item,
      "source_task_fingerprint_sha256",
    ),
    selected_plan_task_ids: strings(item, "selected_plan_task_ids"),
    excluded_source_overlap_task_ids: strings(
      item,
      "excluded_source_overlap_task_ids",
    ),
    reserve_plan_task_ids: strings(item, "reserve_plan_task_ids"),
    ambient_context: strings(item, "ambient_context"),
    summaries: Array.isArray(item.summaries)
      ? item.summaries.map(armSummary)
      : [],
    plan_summaries: Array.isArray(item.plan_summaries)
      ? item.plan_summaries.map(armSummary)
      : [],
    applicability_summaries: Array.isArray(item.applicability_summaries)
      ? item.applicability_summaries.map(armSummary)
      : [],
    trials: Array.isArray(item.trials) ? item.trials.map(trial) : [],
    regressions: strings(item, "regressions"),
    install_allowed: booleanField(item, "install_allowed"),
    gate_reason: stringField(item, "gate_reason"),
  };
}
function installPlan(value: unknown): SkillInstallPlan {
  const item = record(value, "install plan");
  return {
    plan_id: stringField(item, "plan_id"),
    evaluation_id: stringField(item, "evaluation_id"),
    tool: stringField(item, "tool"),
    target_location: stringField(item, "target_location"),
    skill_location: stringField(item, "skill_location"),
    marker_location: stringField(item, "marker_location"),
    occupied: booleanField(item, "occupied"),
    can_install: booleanField(item, "can_install"),
    skill_md: stringField(item, "skill_md"),
    skill_sha256: stringField(item, "skill_sha256"),
    marker_json: stringField(item, "marker_json"),
    marker_file_sha256: stringField(item, "marker_file_sha256"),
  };
}
function installed(value: unknown): InstalledSkill {
  const item = record(value, "installed skill");
  return {
    install_id: stringField(item, "install_id"),
    evaluation_id: stringField(item, "evaluation_id"),
    source_submission_id: stringField(item, "source_submission_id"),
    tool: stringField(item, "tool"),
    name: stringField(item, "name"),
    target_location: stringField(item, "target_location"),
    skill_sha256: stringField(item, "skill_sha256"),
    marker_sha256: stringField(item, "marker_sha256"),
    installed_at: stringField(item, "installed_at"),
  };
}

export async function getSkillCopy(): Promise<SkillCopy> {
  const item = record(
    await invokeTauri<unknown>("skill_learning_copy"),
    "copy",
  );
  const keys = [
    "heading",
    "promise",
    "supported_family",
    "learn_action",
    "learning",
    "candidate_heading",
    "generated_source",
    "name",
    "applicability",
    "procedure",
    "source_evidence",
    "manual_instruction",
    "test_contract",
    "contract_summary_format",
    "review_action",
    "reviewing",
    "exact_package",
    "digest",
    "evaluation_disclosure",
    "review_budget_format",
    "approve_and_test",
    "edit_skill",
    "testing",
    "results",
    "passed_gate",
    "failed_gate",
    "repository_plans",
    "skill_applicability",
    "regressions",
    "no_regressions",
    "inspect_runs",
    "model_and_budget",
    "model_budget_format",
    "open_fixture_source",
    "model_output",
    "edits",
    "commands",
    "checks",
    "baseline",
    "simple_instruction",
    "candidate_skill",
    "passed",
    "failed",
    "preparing",
    "review_install",
    "retry_install",
    "install_preview",
    "install_disclosure",
    "persistent_writes",
    "target_path",
    "skill_file",
    "ownership_marker",
    "marker_digest",
    "exact_marker",
    "coding_tool",
    "install_action",
    "installing",
    "installed",
    "rollback",
    "rolling_back",
    "rollback_disclosure",
    "rollback_incomplete",
    "unavailable",
  ] as const;
  for (const key of keys) stringField(item, key);
  return item as unknown as SkillCopy;
}
export async function getSkillCandidate(submissionId: string) {
  return candidate(
    await invokeTauri<unknown>("skill_candidate", { submissionId }),
  );
}
export async function reviewSkill(
  candidateId: string,
  draftValue: SkillDraft,
  replacesReviewId: string | null,
) {
  return review(
    await invokeTauri<unknown>("skill_review", {
      candidateId,
      draft: draftValue,
      replacesReviewId,
    }),
  );
}
export async function evaluateSkill(reviewId: string, skillSha256: string) {
  return report(
    await invokeTauri<unknown>("skill_evaluate", { reviewId, skillSha256 }),
  );
}
export async function planSkillInstall(evaluationId: string) {
  return installPlan(
    await invokeTauri<unknown>("skill_install_plan", { evaluationId }),
  );
}
export async function commitSkillInstall(plan: SkillInstallPlan) {
  return installed(
    await invokeTauri<unknown>("skill_install_commit", {
      planId: plan.plan_id,
      asPreviewedSha256: plan.skill_sha256,
      asPreviewedMarkerSha256: plan.marker_file_sha256,
    }),
  );
}
export async function getSkillInstallStatus(sourceSubmissionId: string) {
  const value = record(
    await invokeTauri<unknown>("skill_install_status", { sourceSubmissionId }),
    "status",
  );
  if (typeof value.installed !== "boolean")
    throw new Error("Invalid skill installation status");
  return value.installed && value.skill ? installed(value.skill) : null;
}
export async function rollbackSkill(installedSkill: InstalledSkill) {
  const value = record(
    await invokeTauri<unknown>("skill_install_rollback", {
      installId: installedSkill.install_id,
      sourceSubmissionId: installedSkill.source_submission_id,
    }),
    "rollback",
  );
  if (
    typeof value.removed !== "boolean" ||
    typeof value.retained_directory !== "boolean"
  )
    throw new Error("Invalid skill rollback response");
  return value;
}
