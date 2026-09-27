export type HistoryRecord = {
  submission_id: string;
  project_id: string;
  submitted_at: string;
  project_label: string;
  source: string;
  status: string;
  credit_points_pending: number;
  credit_points_final: number | null;
  explanations: string[];
};

export type WithdrawalResult = {
  withdrawn: boolean;
  distribution_reach:
    | "not_distributed"
    | "commons_not_distributed"
    | "commons_distributed"
    | null;
  token_deletion_note: string | null;
};

export type HistoryRollup = {
  week: HistoryCounts;
  month: HistoryCounts;
  all_time: {
    submitted: number;
    accepted: number;
    quarantined: number;
    other: number;
  };
  credit_pending: number;
  credit_final: number;
  last_refreshed_at: string | null;
  community?: CommunityStanding;
};

export type HistoryCounts = {
  submitted: number;
  accepted: number;
  quarantined: number;
  other: number;
};

export type CommunityStanding = {
  rank: number | null;
  novelty_credit: number;
  accepted_in_window: number;
  accept_rate: number | null;
  window_label: string;
  public_since: string | null;
  snapshot_at: string | null;
  analytics_withheld: boolean;
};

export type HistoryData = { history: HistoryRecord[]; rollup: HistoryRollup };

export type HistoryDetail = {
  content_unavailable: boolean;
  task: string | null;
  task_success: string | null;
  contribution_status: string | null;
  permitted_uses: string[];
  human_correction: string | null;
  evidence: Array<{ event_id: string; kind: string; excerpt: string }>;
  contributed_version: string;
  consent_policy_version: string;
  redaction_pipeline_version: string;
  publication_version: number;
  retained_source_slug: string | null;
  publication: PublicRunPage | null;
};

export type PublicRunReusePermission = "cc_by_4_0" | "cc0_1_0";

export type PublicRunEvidenceDraft = { event_id: string; excerpt: string };

export type PublicRunEditorInput = {
  title: string;
  outcome_summary: string;
  correction_excerpt: string | null;
  workflow: string;
  reuse_permission: PublicRunReusePermission | null;
  evidence: PublicRunEvidenceDraft[];
  source: string;
};

export type PublicRunDraft = Omit<
  PublicRunEditorInput,
  "reuse_permission" | "source"
> & {
  reuse_permission: PublicRunReusePermission;
  source_slug: string | null;
};

export type PublicRunPage = {
  slug: string;
  title: string;
  outcome_summary: string;
  correction_excerpt: string | null;
  workflow: string;
  reuse_permission: PublicRunReusePermission;
  evidence: Array<{ excerpt: string }>;
  task_success: string;
  contributed_version: string;
  version: number;
  published_at: string;
  public_url: string | null;
  source: { slug: string; title: string } | null;
  source_unavailable: boolean;
  variations: Array<{ slug: string; title: string }>;
  credential_warning: string | null;
};

export type PublicRunValidation = {
  draft: PublicRunDraft | null;
  error: string | null;
};
