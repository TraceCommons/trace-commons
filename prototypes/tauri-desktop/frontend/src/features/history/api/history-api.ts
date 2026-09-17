import { daemonCall, invokeTauri } from "../../../lib/tauri/core-api";
import type {
  CommunityStanding,
  HistoryCounts,
  HistoryData,
  HistoryDetail,
  HistoryRecord,
  HistoryRollup,
  WithdrawalResult,
} from "../types";
import { parsePublicRunPage } from "./public-run-api";

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null)
    throw new Error("Invalid history response");
  return value as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid history field: ${key}`);
  return value[key] as string;
}

function numberField(value: Record<string, unknown>, key: string) {
  if (typeof value[key] !== "number")
    throw new Error(`Invalid history field: ${key}`);
  return value[key] as number;
}

function parseRecord(value: unknown): HistoryRecord {
  const item = record(value);
  return {
    submission_id: stringField(item, "submission_id"),
    project_id: stringField(item, "project_id"),
    submitted_at: stringField(item, "submitted_at"),
    project_label: stringField(item, "project_label"),
    source: stringField(item, "source"),
    status: stringField(item, "status"),
    credit_points_pending: numberField(item, "credit_points_pending"),
    credit_points_final:
      item.credit_points_final === null
        ? null
        : numberField(item, "credit_points_final"),
    explanations: Array.isArray(item.explanations)
      ? item.explanations.filter(
          (item): item is string => typeof item === "string",
        )
      : [],
  };
}

export async function withdrawHistory(
  submissionId: string,
): Promise<WithdrawalResult> {
  const value = record(
    await invokeTauri<unknown>("withdraw_history", { submissionId }),
  );
  const reach = value.distribution_reach;
  if (
    (value.withdrawn !== undefined && typeof value.withdrawn !== "boolean") ||
    (reach !== undefined && reach !== null && typeof reach !== "string") ||
    (value.token_deletion_note !== undefined &&
      value.token_deletion_note !== null &&
      typeof value.token_deletion_note !== "string")
  )
    throw new Error("Invalid withdrawal response");
  const knownReach =
    reach === "not_distributed" ||
    reach === "commons_not_distributed" ||
    reach === "commons_distributed"
      ? reach
      : null;
  return {
    withdrawn: value.withdrawn !== false,
    distribution_reach: knownReach,
    token_deletion_note:
      value.token_deletion_note === undefined
        ? null
        : (value.token_deletion_note as string | null),
  };
}

function parseRollup(value: unknown): HistoryRollup {
  const item = record(value);
  const counts = (value: unknown) => {
    const row = record(value);
    return {
      submitted: numberField(row, "submitted"),
      accepted: numberField(row, "accepted"),
      quarantined: numberField(row, "quarantined"),
      other: numberField(row, "other"),
    } satisfies HistoryCounts;
  };
  const allTime = record(item.all_time);
  const communityValue = item.community;
  const community =
    communityValue === undefined
      ? undefined
      : (() => {
          const standing = record(communityValue);
          if (typeof standing.analytics_withheld !== "boolean")
            throw new Error("Invalid community standing response");
          return {
            rank: standing.rank === null ? null : numberField(standing, "rank"),
            novelty_credit: numberField(standing, "novelty_credit"),
            accepted_in_window: numberField(standing, "accepted_in_window"),
            accept_rate:
              standing.accept_rate === null
                ? null
                : numberField(standing, "accept_rate"),
            window_label: stringField(standing, "window_label"),
            public_since: nullableString(standing, "public_since"),
            snapshot_at: nullableString(standing, "snapshot_at"),
            analytics_withheld: standing.analytics_withheld,
          } satisfies CommunityStanding;
        })();
  return {
    week: counts(item.week),
    month: counts(item.month),
    all_time: {
      submitted: numberField(allTime, "submitted"),
      accepted: numberField(allTime, "accepted"),
      quarantined: numberField(allTime, "quarantined"),
      other: numberField(allTime, "other"),
    },
    credit_pending: numberField(item, "credit_pending"),
    credit_final: numberField(item, "credit_final"),
    last_refreshed_at: nullableString(item, "last_refreshed_at"),
    community,
  };
}

export async function getHistoryData(): Promise<HistoryData> {
  const [historyResponse, rollupResponse] = await Promise.all([
    daemonCall<unknown>("list_history", { limit: 100 }),
    daemonCall<unknown>("history_rollup"),
  ]);
  const history = record(historyResponse).history;
  if (!Array.isArray(history)) throw new Error("Invalid history response");
  return {
    history: history.map(parseRecord),
    rollup: parseRollup(rollupResponse),
  };
}

function nullableString(value: Record<string, unknown>, key: string) {
  if (value[key] === null || value[key] === undefined) return null;
  return stringField(value, key);
}

export async function getHistoryDetail(
  submissionId: string,
): Promise<HistoryDetail> {
  const value = record(
    await invokeTauri<unknown>("history_detail", { submissionId }),
  );
  const evidence = Array.isArray(value.evidence)
    ? value.evidence.map((entry) => {
        const item = record(entry);
        return {
          event_id: stringField(item, "event_id"),
          kind: stringField(item, "kind"),
          excerpt: stringField(item, "excerpt"),
        };
      })
    : [];
  const publicationValue = value.publication;
  const publication =
    publicationValue === null || publicationValue === undefined
      ? null
      : parsePublicRunPage(publicationValue);
  if (
    typeof value.content_unavailable !== "boolean" ||
    (value.permitted_uses !== undefined &&
      (!Array.isArray(value.permitted_uses) ||
        !value.permitted_uses.every((item) => typeof item === "string"))) ||
    (value.publication_version !== undefined &&
      (typeof value.publication_version !== "number" ||
        !Number.isInteger(value.publication_version)))
  )
    throw new Error("Invalid history detail response");
  return {
    content_unavailable: value.content_unavailable,
    task: nullableString(value, "task"),
    task_success: nullableString(value, "task_success"),
    contribution_status: nullableString(value, "contribution_status"),
    permitted_uses: (value.permitted_uses ?? []) as string[],
    human_correction: nullableString(value, "human_correction"),
    evidence,
    contributed_version: stringField(value, "contributed_version"),
    consent_policy_version: stringField(value, "consent_policy_version"),
    redaction_pipeline_version: stringField(
      value,
      "redaction_pipeline_version",
    ),
    publication_version:
      typeof value.publication_version === "number"
        ? value.publication_version
        : 0,
    retained_source_slug: nullableString(value, "retained_source_slug"),
    publication,
  };
}
