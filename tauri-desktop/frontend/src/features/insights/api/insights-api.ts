import { invokeTauri, invokeTauriBytes } from "../../../lib/tauri/core-api";
import type {
  Insight,
  InsightMetric,
  InsightOutcomeLink,
  InsightSummary,
  InsightsData,
} from "../types";

type RecordValue = Record<string, unknown>;

function record(value: unknown, label: string): RecordValue {
  if (typeof value !== "object" || value === null)
    throw new Error(`Invalid Insights ${label}`);
  return value as RecordValue;
}

function string(value: RecordValue, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid Insights field: ${key}`);
  return value[key] as string;
}

function number(value: RecordValue, key: string) {
  if (typeof value[key] !== "number")
    throw new Error(`Invalid Insights field: ${key}`);
  return value[key] as number;
}

function optionalNumber(value: RecordValue, key: string) {
  if (value[key] === null) return null;
  return number(value, key);
}

function strings(value: unknown, label: string) {
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string"))
    throw new Error(`Invalid Insights ${label}`);
  return value as string[];
}

function nullableString(value: RecordValue, key: string) {
  if (value[key] === null || value[key] === undefined) return null;
  return string(value, key);
}

function parseOutcomeLink(value: unknown): InsightOutcomeLink {
  const item = record(value, "outcome link");
  const evidence = record(item.evidence, "outcome evidence");
  const type = string(evidence, "type");
  if (type === "git_commit") {
    const git = record(evidence.evidence, "Git evidence");
    return {
      id: string(item, "id"),
      source_digest: string(item, "source_digest"),
      linked_at: string(item, "linked_at"),
      provenance: string(item, "provenance"),
      evidence: {
        type,
        evidence: {
          repository_path_digest: string(git, "repository_path_digest"),
          object_id: string(git, "object_id"),
          tree_id: string(git, "tree_id"),
          parent_ids: strings(git.parent_ids, "Git parents"),
          inspected_at: string(git, "inspected_at"),
          provenance: string(git, "provenance"),
        },
      },
    };
  }
  if (type === "test_report") {
    const report = record(evidence.evidence, "test report evidence");
    return {
      id: string(item, "id"),
      source_digest: string(item, "source_digest"),
      linked_at: string(item, "linked_at"),
      provenance: string(item, "provenance"),
      evidence: {
        type,
        evidence: {
          schema_version: number(report, "schema_version"),
          runner: string(report, "runner"),
          passed: number(report, "passed"),
          failed: number(report, "failed"),
          skipped: number(report, "skipped"),
          observed_at: string(report, "observed_at"),
          commit_id: nullableString(report, "commit_id"),
          artifact_digest: string(report, "artifact_digest"),
          imported_at: string(report, "imported_at"),
          provenance: string(report, "provenance"),
        },
      },
    };
  }
  throw new Error("Invalid Insights outcome evidence type");
}

function parseMetric(value: unknown): InsightMetric {
  const item = record(value, "metric");
  const coverage = record(item.coverage, "coverage");
  return {
    id: string(item, "id"),
    value: optionalNumber(item, "value"),
    coverage: {
      observed: number(coverage, "observed"),
      total: number(coverage, "total"),
    },
    evidence_ids: strings(item.evidence_ids, "metric evidence"),
  };
}

export function parseInsight(value: unknown): Insight {
  const item = record(value, "snapshot");
  const report = record(item.report, "report");
  const provider = record(report.provider, "provider");
  const evidence = Array.isArray(report.evidence)
    ? report.evidence.map((entry) => {
        const parsed = record(entry, "evidence");
        return {
          id: string(parsed, "id"),
          source_digest: string(parsed, "source_digest"),
        };
      })
    : [];
  const annotation =
    item.manual_annotation === null
      ? null
      : item.manual_annotation === undefined
        ? null
        : record(item.manual_annotation, "annotation");
  return {
    id: string(item, "id"),
    source_format: string(item, "source_format"),
    boundary: string(item, "boundary"),
    analyzed_at: string(item, "analyzed_at"),
    report: {
      schema_version: number(report, "schema_version"),
      provider: {
        id: string(provider, "id"),
        version: string(provider, "version"),
        rubric_version: string(provider, "rubric_version"),
        execution_mode: string(provider, "execution_mode"),
      },
      metrics: Array.isArray(report.metrics)
        ? report.metrics.map(parseMetric)
        : [],
      evidence,
    },
    estimated_cost_usd: optionalNumber(item, "estimated_cost_usd"),
    cost_unavailable_reason: string(item, "cost_unavailable_reason"),
    manual_annotation: annotation
      ? {
          category: string(annotation, "category"),
          outcome: string(annotation, "outcome"),
          provenance: string(annotation, "provenance"),
          recorded_at: string(annotation, "recorded_at"),
        }
      : null,
    outcome_links: Array.isArray(item.outcome_links)
      ? item.outcome_links.map(parseOutcomeLink)
      : [],
  };
}

function parseSummary(value: unknown): InsightSummary {
  const item = record(value, "summary");
  const userReported = record(item.user_reported, "assessments");
  const categories = Array.isArray(item.categories)
    ? item.categories.map((entry) => {
        const parsed = record(entry, "category");
        return {
          category: string(parsed, "category"),
          snapshots: number(parsed, "snapshots"),
        };
      })
    : [];
  const outcomes = Array.isArray(item.outcomes)
    ? item.outcomes.map((entry) => {
        const parsed = record(entry, "outcome");
        return {
          outcome: string(parsed, "outcome"),
          snapshots: number(parsed, "snapshots"),
        };
      })
    : [];
  const metrics = Array.isArray(item.metrics)
    ? item.metrics.map((entry) => {
        const parsed = record(entry, "summary metric");
        return {
          id: string(parsed, "id"),
          observed_value_sum: optionalNumber(parsed, "observed_value_sum"),
          available_snapshots: number(parsed, "available_snapshots"),
          missing_snapshots: number(parsed, "missing_snapshots"),
        };
      })
    : [];
  return {
    saved_snapshots: number(item, "saved_snapshots"),
    assessed_snapshots: number(userReported, "assessed_snapshots"),
    unassessed_snapshots: number(userReported, "unassessed_snapshots"),
    categories,
    outcomes,
    metrics,
    limitations: strings(item.limitations, "limitations"),
  };
}

export async function getInsightsData(): Promise<InsightsData> {
  const [listResponse, summaryResponse] = await Promise.all([
    invokeTauri("insights_list"),
    invokeTauri("insights_summary"),
  ]);
  const list = record(listResponse, "list");
  if (!Array.isArray(list.insights)) throw new Error("Invalid Insights list");
  const summary = record(summaryResponse, "summary response");
  if (!summary.summary) throw new Error("Invalid Insights summary");
  return {
    snapshots: list.insights.map(parseInsight),
    summary: parseSummary(summary.summary),
  };
}

export async function analyzeInsight(
  file: File,
  source: string,
  save: boolean,
) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const response = record(
    await invokeTauriBytes("analyze_insight", bytes, {
      "x-tc-source": source,
      "x-tc-save": String(save),
    }),
    "analyze",
  );
  return parseInsight(response.insight);
}

export async function explainInsight(id: string) {
  const response = record(
    await invokeTauri("explain_insight", { id }),
    "explain",
  );
  return parseInsight(response.insight);
}

export async function deleteInsight(id: string) {
  const response = record(
    await invokeTauri("delete_insight", { id }),
    "delete",
  );
  if (response.deleted !== true) throw new Error("Insight was not deleted");
}

export async function annotateInsight(
  id: string,
  category: string,
  outcome: string,
) {
  const response = record(
    await invokeTauri("annotate_insight", { id, category, outcome }),
    "assessment",
  );
  return parseInsight(response.insight);
}

export async function clearInsightAnnotation(id: string) {
  const response = record(
    await invokeTauri("clear_insight_annotation", { id }),
    "assessment",
  );
  return parseInsight(response.insight);
}

export async function linkTestReport(id: string, file: File) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const response = record(
    await invokeTauriBytes("link_test_report", bytes, {
      "x-tc-insight-id": id,
    }),
    "test report evidence",
  );
  return parseInsight(response.insight);
}

export async function linkGit(id: string, repository: string, commit: string) {
  const response = record(
    await invokeTauri("link_git_insight", { id, repository, commit }),
    "Git evidence",
  );
  return parseInsight(response.insight);
}

export async function unlinkEvidence(id: string, evidenceId: string) {
  const response = record(
    await invokeTauri("unlink_insight_evidence", {
      id,
      evidenceId,
    }),
    "unlink evidence",
  );
  return parseInsight(response.insight);
}
