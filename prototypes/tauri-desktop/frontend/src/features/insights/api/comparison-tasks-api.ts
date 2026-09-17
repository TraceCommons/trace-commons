import { invokeTauri } from "../../../lib/tauri/core-api";
import type {
  ComparisonContext,
  ComparisonTask,
  ComparisonTaskDetail,
  FrozenEpisode,
} from "../comparisons";

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
const nullableString = (value: RecordValue, key: string) =>
  value[key] === null ? null : string(value, key);

function parseEpisode(value: unknown): FrozenEpisode {
  const item = record(value, "frozen episode");
  if (!Array.isArray(item.members))
    throw new Error("Invalid comparison episode members");
  return {
    episode_id: string(item, "episode_id"),
    revision: number(item, "revision"),
    membership_revision: number(item, "membership_revision"),
    members_digest: string(item, "members_digest"),
    members: item.members.map((member) => {
      const parsed = record(member, "frozen member");
      return {
        snapshot_id: string(parsed, "snapshot_id"),
        source_digest: string(parsed, "source_digest"),
      };
    }),
  };
}

function parseContext(value: unknown): ComparisonContext {
  const item = record(value, "context");
  const checkout = record(item.checkout_provenance, "checkout provenance");
  const language = record(item.language, "language");
  const configuration = record(item.configuration, "configuration");
  return {
    project_id: string(item, "project_id"),
    category: string(item, "category"),
    task_date: string(item, "task_date"),
    checkout_provenance: {
      state: string(checkout, "state"),
      scheme:
        checkout.scheme === undefined ? undefined : string(checkout, "scheme"),
      digest:
        checkout.digest === undefined ? undefined : string(checkout, "digest"),
    },
    language: {
      state: string(language, "state"),
      value:
        language.value === undefined ? undefined : string(language, "value"),
    },
    configuration,
    configuration_fingerprint: string(item, "configuration_fingerprint"),
  };
}

function parseTask(value: unknown): ComparisonTask {
  const item = record(value, "task");
  if (!Array.isArray(item.episodes))
    throw new Error("Invalid comparison task episodes");
  const outcome =
    item.outcome === null
      ? null
      : (() => {
          const parsed = record(item.outcome, "outcome");
          return {
            value: string(parsed, "value"),
            material_revision: number(parsed, "material_revision"),
            material_digest: string(parsed, "material_digest"),
            recorded_at: string(parsed, "recorded_at"),
          };
        })();
  const confirmation =
    item.independence_confirmation === null
      ? null
      : (() => {
          const parsed = record(item.independence_confirmation, "confirmation");
          return {
            material_revision: number(parsed, "material_revision"),
            material_digest: string(parsed, "material_digest"),
            confirmed_at: string(parsed, "confirmed_at"),
          };
        })();
  return {
    schema_version: number(item, "schema_version"),
    id: string(item, "id"),
    revision: number(item, "revision"),
    material_revision: number(item, "material_revision"),
    material_digest: string(item, "material_digest"),
    created_at: string(item, "created_at"),
    updated_at: string(item, "updated_at"),
    material_recorded_at: nullableString(item, "material_recorded_at"),
    episodes: item.episodes.map(parseEpisode),
    context: item.context === null ? null : parseContext(item.context),
    outcome,
    independence_confirmation: confirmation,
  };
}

function parseDetail(value: unknown): ComparisonTaskDetail {
  const item = record(value, "task detail");
  const qualification =
    item.source_qualification === null
      ? null
      : {
          declared_model_cohort: string(
            record(item.source_qualification, "qualification"),
            "declared_model_cohort",
          ),
        };
  const stale =
    Array.isArray(item.stale_reasons) &&
    item.stale_reasons.every((entry) => typeof entry === "string")
      ? (item.stale_reasons as string[])
      : (() => {
          throw new Error("Invalid comparison stale reasons");
        })();
  const overlaps =
    Array.isArray(item.overlapping_task_ids) &&
    item.overlapping_task_ids.every((entry) => typeof entry === "string")
      ? (item.overlapping_task_ids as string[])
      : (() => {
          throw new Error("Invalid comparison overlaps");
        })();
  return {
    task: parseTask(item.task),
    source_qualification: qualification,
    stale_reasons: stale,
    overlapping_task_ids: overlaps,
    resolved_at: string(item, "resolved_at"),
  };
}

export async function listComparisonTasks(): Promise<ComparisonTaskDetail[]> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_list"),
    "task list",
  );
  if (!Array.isArray(response.tasks))
    throw new Error("Invalid comparison task list");
  return response.tasks.map(parseDetail);
}
export async function explainComparisonTask(
  id: string,
): Promise<ComparisonTaskDetail> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_explain", { id }),
    "task detail",
  );
  return parseDetail(response.detail);
}
export async function createComparisonTask(
  episodeIds: string[],
): Promise<ComparisonTask> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_create", { episodeIds }),
    "task create",
  );
  return parseTask(response.task);
}
export async function replaceComparisonTaskEpisodes(
  id: string,
  expectedRevision: number,
  episodeIds: string[],
): Promise<ComparisonTask> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_replace_episodes", {
      id,
      expectedRevision,
      episodeIds,
    }),
    "task episodes",
  );
  return parseTask(response.task);
}
export async function setComparisonTaskContext(
  id: string,
  expectedRevision: number,
  context: ComparisonContext,
): Promise<ComparisonTask> {
  const { configuration_fingerprint: _fingerprint, ...input } = context;
  const response = record(
    await invokeTauri<unknown>("comparison_task_set_context", {
      id,
      expectedRevision,
      context: input,
    }),
    "task context",
  );
  return parseTask(response.task);
}
export async function setComparisonTaskOutcome(
  id: string,
  expectedRevision: number,
  value: string,
): Promise<ComparisonTask> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_set_outcome", {
      id,
      expectedRevision,
      value,
    }),
    "task outcome",
  );
  return parseTask(response.task);
}
export async function clearComparisonTaskOutcome(
  id: string,
  expectedRevision: number,
): Promise<ComparisonTask> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_clear_outcome", {
      id,
      expectedRevision,
    }),
    "task outcome",
  );
  return parseTask(response.task);
}
export async function reconfirmComparisonTask(
  id: string,
  expectedRevision: number,
  displayedMaterialDigest: string,
): Promise<ComparisonTask> {
  const response = record(
    await invokeTauri<unknown>("comparison_task_reconfirm", {
      id,
      expectedRevision,
      displayedMaterialDigest,
    }),
    "task confirmation",
  );
  return parseTask(response.task);
}
