import { invokeTauri } from "../../../lib/tauri/core-api";
import type {
  ComparisonResult,
  ComparisonSpecification,
  ComparisonSpecificationInput,
} from "../specifications";

type RecordValue = Record<string, unknown>;
const record = (value: unknown, label: string): RecordValue => {
  if (typeof value !== "object" || value === null)
    throw new Error(`Invalid comparison ${label}`);
  return value as RecordValue;
};
const string = (value: RecordValue, key: string) => {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid comparison field: ${key}`);
  return value[key] as string;
};
const number = (value: RecordValue, key: string) => {
  if (typeof value[key] !== "number")
    throw new Error(`Invalid comparison field: ${key}`);
  return value[key] as number;
};
const strings = (value: unknown, label: string) => {
  if (
    !Array.isArray(value) ||
    !value.every((entry) => typeof entry === "string")
  )
    throw new Error(`Invalid comparison ${label}`);
  return value as string[];
};
function parseInput(item: RecordValue): ComparisonSpecificationInput {
  const stratum = record(item.stratum, "stratum");
  return {
    evidence_cutoff: string(item, "evidence_cutoff"),
    cohort_labels: strings(item.cohort_labels, "cohorts"),
    date_start: string(item, "date_start"),
    date_end: string(item, "date_end"),
    stratum: {
      project_id: string(stratum, "project_id"),
      language: string(stratum, "language"),
      configuration_fingerprint: string(stratum, "configuration_fingerprint"),
    },
  };
}
function parseSpecification(value: unknown): ComparisonSpecification {
  const item = record(value, "specification");
  const estimator = record(item.estimator_state, "estimator");
  return {
    ...parseInput(item),
    schema_version: number(item, "schema_version"),
    id: string(item, "id"),
    provenance: string(item, "provenance"),
    created_at: string(item, "created_at"),
    category: string(item, "category"),
    task_rule: string(item, "task_rule"),
    attempt_rule: string(item, "attempt_rule"),
    outcome_rubric_version: string(item, "outcome_rubric_version"),
    estimands: strings(item.estimands, "estimands"),
    estimator_state: { status: string(estimator, "status") },
    specification_digest: string(item, "specification_digest"),
    saved_record_digest: string(item, "saved_record_digest"),
  };
}
function parseResult(value: unknown): ComparisonResult {
  const item = record(value, "result");
  if (
    !Array.isArray(item.included_task_ids) ||
    !item.included_task_ids.every((entry) => typeof entry === "string")
  )
    throw new Error("Invalid comparison included tasks");
  if (!Array.isArray(item.excluded_tasks) || !Array.isArray(item.cohorts))
    throw new Error("Invalid comparison result");
  return {
    schema_version: number(item, "schema_version"),
    specification_id: string(item, "specification_id"),
    specification_digest: string(item, "specification_digest"),
    specification_record_digest: string(item, "specification_record_digest"),
    audit_digest: string(item, "audit_digest"),
    estimation_input_digest: string(item, "estimation_input_digest"),
    included_task_ids: item.included_task_ids as string[],
    excluded_tasks: item.excluded_tasks.map((entry) => {
      const excluded = record(entry, "excluded task");
      return {
        task_id: string(excluded, "task_id"),
        reasons: strings(excluded.reasons, "exclusion reasons"),
      };
    }),
    cohorts: item.cohorts.map((entry) => {
      const cohort = record(entry, "cohort");
      const outcomes = record(cohort.outcomes, "outcomes");
      const usage = record(cohort.usage, "usage");
      return {
        cohort_label: string(cohort, "cohort_label"),
        included_tasks: number(cohort, "included_tasks"),
        outcomes: Object.fromEntries(
          Object.entries(outcomes).map(([key, value]) => [
            key,
            typeof value === "number" ? value : 0,
          ]),
        ),
        usage: Object.fromEntries(
          Object.entries(usage).map(([key, value]) => [
            key,
            typeof value === "number" ? value : 0,
          ]),
        ),
      };
    }),
  };
}

export async function listComparisonSpecifications(): Promise<
  ComparisonSpecification[]
> {
  const response = record(
    await invokeTauri<unknown>("comparison_specification_list"),
    "specification list",
  );
  if (!Array.isArray(response.specifications))
    throw new Error("Invalid comparison specifications");
  return response.specifications.map(parseSpecification);
}
export async function previewComparisonSpecification(
  input: ComparisonSpecificationInput,
): Promise<{
  specification: ComparisonSpecification;
  result: ComparisonResult;
}> {
  const response = record(
    await invokeTauri<unknown>("comparison_specification_preview", { input }),
    "specification preview",
  );
  return {
    specification: parseSpecification(response.specification),
    result: parseResult(response.result),
  };
}
export async function saveComparisonSpecification(
  input: ComparisonSpecificationInput,
): Promise<ComparisonSpecification> {
  const response = record(
    await invokeTauri<unknown>("comparison_specification_save", { input }),
    "specification save",
  );
  return parseSpecification(response.specification);
}
export async function evaluateComparisonSpecification(
  id: string,
): Promise<ComparisonResult> {
  const response = record(
    await invokeTauri<unknown>("comparison_specification_evaluate", { id }),
    "specification result",
  );
  return parseResult(response.result);
}
