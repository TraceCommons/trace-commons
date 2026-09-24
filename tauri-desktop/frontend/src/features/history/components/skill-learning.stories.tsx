import type { Meta, StoryObj } from "@storybook/react-vite";
import type {
  InstalledSkill,
  SkillCandidate,
  SkillCopy,
  SkillInstallPlan,
  SkillReview,
} from "../skill-types";
import { SkillCandidateForm } from "./skill-candidate-form";
import { SkillInstallPreview } from "./skill-install-preview";
import { SkillReviewPreview } from "./skill-review-preview";

const copy: SkillCopy = {
  heading: "Learn from session",
  promise: "Turn this correction into an Agent Skill.",
  supported_family: "Generated source repair",
  learn_action: "Learn from session",
  learning: "Learning…",
  candidate_heading: "Candidate skill",
  generated_source: "Generated source",
  name: "Skill name",
  applicability: "When it applies",
  procedure: "Corrective procedure",
  source_evidence: "Source evidence",
  manual_instruction: "Edit source of truth, then regenerate.",
  test_contract: "Held-out plan test",
  contract_summary_format:
    "%1$d tasks across %2$d debugging cases. %3$d requests compare the baseline, a simple instruction, and this candidate. Each request allows %4$d output tokens and %5$d seconds.",
  review_action: "Review skill",
  reviewing: "Reviewing…",
  exact_package: "Exact installable package",
  digest: "SHA-256",
  evaluation_disclosure:
    "Evaluation uses public fixtures and the package shown above.",
  review_budget_format: "%1$d model requests",
  approve_and_test: "Approve and test",
  edit_skill: "Edit skill",
  testing: "Testing…",
  results: "Held-out plan results",
  passed_gate: "Passed plan gate",
  failed_gate: "Needs revision",
  repository_plans: "Repository plans",
  skill_applicability: "Skill applicability",
  regressions: "Regressions",
  no_regressions: "No regressions.",
  inspect_runs: "Inspect all runs",
  model_and_budget: "Model and budget",
  model_budget_format: "Owner %1$@",
  open_fixture_source: "Open fixture source",
  model_output: "Full model output",
  edits: "Direct source edits",
  commands: "Commands",
  checks: "Checks",
  baseline: "Baseline",
  simple_instruction: "Simple instruction",
  candidate_skill: "Candidate skill",
  passed: "Passed",
  failed: "Failed",
  preparing: "Preparing…",
  review_install: "Review install",
  retry_install: "Retry install",
  install_preview: "Installation preview",
  install_disclosure: "Creates two persistent files.",
  persistent_writes: "Persistent writes",
  target_path: "Target path",
  skill_file: "Skill file",
  ownership_marker: "Ownership marker",
  marker_digest: "Marker file SHA-256",
  exact_marker: "Exact ownership marker",
  coding_tool: "Coding tool",
  install_action: "Install skill",
  installing: "Installing…",
  installed: "Installed in Codex",
  rollback: "Roll back",
  rolling_back: "Rolling back…",
  rollback_disclosure: "Rollback checks ownership.",
  rollback_incomplete: "Rollback incomplete.",
  unavailable: "Unavailable",
};

const candidate: SkillCandidate = {
  candidate_id: "candidate-1",
  family: "generated-source-repair",
  source_submission_id: "submission-1",
  source_correction: "Edit source schema, then regenerate the derived file.",
  source_evidence: [
    {
      event_id: "event-1",
      kind: "correction",
      excerpt: "The generated output should not be edited directly.",
    },
  ],
  draft: {
    name: "repair-generated-sources",
    description: "Use when a debugging task affects generated files.",
    procedure:
      "1. Find authoritative source.\n2. Regenerate outputs.\n3. Run drift checks.",
  },
  replaces_review_id: null,
  manual_control_instruction: "Edit source of truth, then regenerate.",
  evaluation_contract: {
    task_count: 6,
    plan_task_count: 4,
    fixture_pool_count: 8,
    cluster_count: 3,
    arm_count: 3,
    total_requests: 18,
    output_token_limit: 900,
    request_timeout_seconds: 90,
    required_model_owner: "nearai",
    fixture_scope: "Public fixtures",
    selection_policy: "Exclude source overlap.",
  },
};
const review: SkillReview = {
  review_id: "review-1",
  candidate_id: "candidate-1",
  source_submission_id: "submission-1",
  source_evidence_ids: ["event-1"],
  draft: candidate.draft,
  skill_md:
    "---\nname: repair-generated-sources\n---\n\n# Repair generated sources",
  skill_sha256: "a".repeat(64),
};
const plan: SkillInstallPlan = {
  plan_id: "plan-1",
  evaluation_id: "evaluation-1",
  tool: "codex",
  target_location: "~/.codex/skills/repair-generated-sources",
  skill_location: "~/.codex/skills/repair-generated-sources/SKILL.md",
  marker_location:
    "~/.codex/skills/repair-generated-sources/.trace-commons-owner.json",
  occupied: false,
  can_install: true,
  skill_md: review.skill_md,
  skill_sha256: review.skill_sha256,
  marker_json: '{"schema_version":1}',
  marker_file_sha256: "b".repeat(64),
};
const installed: InstalledSkill = {
  install_id: "install-1",
  evaluation_id: "evaluation-1",
  source_submission_id: "submission-1",
  tool: "codex",
  name: "repair-generated-sources",
  target_location: plan.target_location,
  skill_sha256: plan.skill_sha256,
  marker_sha256: plan.marker_file_sha256,
  installed_at: "2026-09-15T10:00:00Z",
};

const meta = {
  title: "History/Skill learning",
  parameters: { layout: "centered" },
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

export const Candidate: Story = {
  render: () => (
    <SkillCandidateForm
      candidate={candidate}
      copy={copy}
      busy={false}
      onReview={() => undefined}
    />
  ),
};
export const ReviewedPackage: Story = {
  render: () => (
    <SkillReviewPreview
      copy={copy}
      review={review}
      busy={false}
      onEdit={() => undefined}
      onApprove={() => undefined}
    />
  ),
};
export const InstallationPreview: Story = {
  render: () => (
    <SkillInstallPreview
      copy={copy}
      plan={plan}
      installed={null}
      busy={false}
      onInstall={() => undefined}
      onRollback={() => undefined}
    />
  ),
};
export const Installed: Story = {
  render: () => (
    <SkillInstallPreview
      copy={copy}
      plan={plan}
      installed={installed}
      busy={false}
      onInstall={() => undefined}
      onRollback={() => undefined}
    />
  ),
};
