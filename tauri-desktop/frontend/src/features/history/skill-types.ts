export type SkillDraft = {
  name: string;
  description: string;
  procedure: string;
};

export type SkillCopy = {
  heading: string;
  promise: string;
  supported_family: string;
  learn_action: string;
  learning: string;
  candidate_heading: string;
  generated_source: string;
  name: string;
  applicability: string;
  procedure: string;
  source_evidence: string;
  manual_instruction: string;
  test_contract: string;
  contract_summary_format: string;
  review_action: string;
  reviewing: string;
  exact_package: string;
  digest: string;
  evaluation_disclosure: string;
  review_budget_format: string;
  approve_and_test: string;
  edit_skill: string;
  testing: string;
  results: string;
  passed_gate: string;
  failed_gate: string;
  repository_plans: string;
  skill_applicability: string;
  regressions: string;
  no_regressions: string;
  inspect_runs: string;
  model_and_budget: string;
  model_budget_format: string;
  open_fixture_source: string;
  model_output: string;
  edits: string;
  commands: string;
  checks: string;
  baseline: string;
  simple_instruction: string;
  candidate_skill: string;
  passed: string;
  failed: string;
  preparing: string;
  review_install: string;
  retry_install: string;
  install_preview: string;
  install_disclosure: string;
  persistent_writes: string;
  target_path: string;
  skill_file: string;
  ownership_marker: string;
  marker_digest: string;
  exact_marker: string;
  coding_tool: string;
  install_action: string;
  installing: string;
  installed: string;
  rollback: string;
  rolling_back: string;
  rollback_disclosure: string;
  rollback_incomplete: string;
  unavailable: string;
};

export type SkillEvaluationContract = {
  task_count: number;
  plan_task_count: number;
  fixture_pool_count: number;
  cluster_count: number;
  arm_count: number;
  total_requests: number;
  output_token_limit: number;
  request_timeout_seconds: number;
  required_model_owner: string;
  fixture_scope: string;
  selection_policy: string;
};

export type SkillSourceEvidence = {
  event_id: string;
  kind: string;
  excerpt: string;
};

export type SkillCandidate = {
  candidate_id: string;
  family: string;
  source_submission_id: string;
  source_correction: string;
  source_evidence: SkillSourceEvidence[];
  draft: SkillDraft;
  replaces_review_id: string | null;
  manual_control_instruction: string;
  evaluation_contract: SkillEvaluationContract;
};

export type SkillReview = {
  review_id: string;
  candidate_id: string;
  source_submission_id: string;
  source_evidence_ids: string[];
  draft: SkillDraft;
  skill_md: string;
  skill_sha256: string;
};

export type SkillArmSummary = {
  arm: string;
  label: string;
  passed: number;
  total: number;
};
export type SkillTrial = {
  task_id: string;
  cluster: string;
  task: string;
  source_url: string;
  arm: string;
  passed: boolean;
  failure_reasons: string[];
  answer: {
    diagnosis: string;
    edit_paths: string[];
    commands: string[];
    verification: string[];
  } | null;
  raw_output: string;
  request_id: string | null;
  served_model: string;
  finish_reason: string;
  usage: {
    prompt_tokens: number | null;
    completion_tokens: number | null;
    reasoning_tokens: number | null;
    total_tokens: number | null;
  };
};

export type SkillEvaluationReport = {
  evaluation_id: string;
  review_id: string;
  skill_sha256: string;
  evaluation_contract: SkillEvaluationContract;
  requested_model: string;
  served_model: string;
  model_owned_by: string;
  source_task_fingerprint_sha256: string;
  selected_plan_task_ids: string[];
  excluded_source_overlap_task_ids: string[];
  reserve_plan_task_ids: string[];
  ambient_context: string[];
  summaries: SkillArmSummary[];
  plan_summaries: SkillArmSummary[];
  applicability_summaries: SkillArmSummary[];
  trials: SkillTrial[];
  regressions: string[];
  install_allowed: boolean;
  gate_reason: string;
};

export type SkillInstallPlan = {
  plan_id: string;
  evaluation_id: string;
  tool: string;
  target_location: string;
  skill_location: string;
  marker_location: string;
  occupied: boolean;
  can_install: boolean;
  skill_md: string;
  skill_sha256: string;
  marker_json: string;
  marker_file_sha256: string;
};

export type InstalledSkill = {
  install_id: string;
  evaluation_id: string;
  source_submission_id: string;
  tool: string;
  name: string;
  target_location: string;
  skill_sha256: string;
  marker_sha256: string;
  installed_at: string;
};
