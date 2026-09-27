export type FrozenEpisode = {
  episode_id: string;
  revision: number;
  membership_revision: number;
  members_digest: string;
  members: Array<{ snapshot_id: string; source_digest: string }>;
};
export type ComparisonContext = {
  project_id: string;
  category: string;
  task_date: string;
  checkout_provenance: { state: string; scheme?: string; digest?: string };
  language: { state: string; value?: string };
  configuration: Record<string, unknown>;
  configuration_fingerprint: string;
};
export type ComparisonTask = {
  schema_version: number;
  id: string;
  revision: number;
  material_revision: number;
  material_digest: string;
  created_at: string;
  updated_at: string;
  material_recorded_at: string | null;
  episodes: FrozenEpisode[];
  context: ComparisonContext | null;
  outcome: {
    value: string;
    material_revision: number;
    material_digest: string;
    recorded_at: string;
  } | null;
  independence_confirmation: {
    material_revision: number;
    material_digest: string;
    confirmed_at: string;
  } | null;
};
export type ComparisonTaskDetail = {
  task: ComparisonTask;
  source_qualification: { declared_model_cohort: string } | null;
  stale_reasons: string[];
  overlapping_task_ids: string[];
  resolved_at: string;
};
