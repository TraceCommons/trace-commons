export type ComparisonSpecificationInput = {
  evidence_cutoff: string;
  cohort_labels: string[];
  date_start: string;
  date_end: string;
  stratum: {
    project_id: string;
    language: string;
    configuration_fingerprint: string;
  };
};
export type ComparisonSpecification = ComparisonSpecificationInput & {
  schema_version: number;
  id: string;
  provenance: string;
  created_at: string;
  category: string;
  task_rule: string;
  attempt_rule: string;
  outcome_rubric_version: string;
  estimands: string[];
  estimator_state: { status: string };
  specification_digest: string;
  saved_record_digest: string;
};
export type ComparisonResult = {
  schema_version: number;
  specification_id: string;
  specification_digest: string;
  specification_record_digest: string;
  audit_digest: string;
  estimation_input_digest: string;
  included_task_ids: string[];
  excluded_tasks: Array<{ task_id: string; reasons: string[] }>;
  cohorts: Array<{
    cohort_label: string;
    included_tasks: number;
    outcomes: Record<string, number>;
    usage: Record<string, number>;
  }>;
};
