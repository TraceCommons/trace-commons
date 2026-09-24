export type InsightMetric = {
  id: string;
  value: number | null;
  coverage: { observed: number; total: number };
  evidence_ids: string[];
};

export type InsightOutcomeLink = {
  id: string;
  source_digest: string;
  linked_at: string;
  provenance: string;
  evidence:
    | {
        type: "git_commit";
        evidence: {
          repository_path_digest: string;
          object_id: string;
          tree_id: string;
          parent_ids: string[];
          inspected_at: string;
          provenance: string;
        };
      }
    | {
        type: "test_report";
        evidence: {
          schema_version: number;
          runner: string;
          passed: number;
          failed: number;
          skipped: number;
          observed_at: string;
          commit_id: string | null;
          artifact_digest: string;
          imported_at: string;
          provenance: string;
        };
      };
};

export type Insight = {
  id: string;
  source_format: string;
  boundary: string;
  analyzed_at: string;
  report: {
    schema_version: number;
    provider: {
      id: string;
      version: string;
      rubric_version: string;
      execution_mode: string;
    };
    metrics: InsightMetric[];
    evidence: Array<{ id: string; source_digest: string }>;
  };
  estimated_cost_usd: number | null;
  cost_unavailable_reason: string;
  manual_annotation: {
    category: string;
    outcome: string;
    provenance: string;
    recorded_at: string;
  } | null;
  outcome_links?: InsightOutcomeLink[];
};

export type InsightSummary = {
  saved_snapshots: number;
  assessed_snapshots: number;
  unassessed_snapshots: number;
  categories: Array<{ category: string; snapshots: number }>;
  outcomes: Array<{ outcome: string; snapshots: number }>;
  metrics: Array<{
    id: string;
    observed_value_sum: number | null;
    available_snapshots: number;
    missing_snapshots: number;
  }>;
  limitations: string[];
};

export type InsightsData = { snapshots: Insight[]; summary: InsightSummary };
