import { invokeTauri } from "../../../lib/tauri/core-api";
import type {
  Episode,
  EpisodeDetail,
  EpisodeListEntry,
  QuestionCardResult,
} from "../workflows";
import { parseInsight } from "./insights-api";

type RecordValue = Record<string, unknown>;
const record = (value: unknown, label: string): RecordValue => {
  if (typeof value !== "object" || value === null)
    throw new Error(`Invalid Insights ${label}`);
  return value as RecordValue;
};
const string = (value: RecordValue, key: string) => {
  if (typeof value[key] !== "string")
    throw new Error(`Invalid Insights field: ${key}`);
  return value[key] as string;
};
const number = (value: RecordValue, key: string) => {
  if (typeof value[key] !== "number")
    throw new Error(`Invalid Insights field: ${key}`);
  return value[key] as number;
};
const strings = (value: unknown, label: string) => {
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string"))
    throw new Error(`Invalid Insights ${label}`);
  return value as string[];
};

function parseEpisode(value: unknown): Episode {
  const item = record(value, "episode");
  if (!Array.isArray(item.members))
    throw new Error("Invalid Insights episode members");
  return {
    schema_version: number(item, "schema_version"),
    id: string(item, "id"),
    revision: number(item, "revision"),
    membership_revision: number(item, "membership_revision"),
    created_at: string(item, "created_at"),
    updated_at: string(item, "updated_at"),
    provenance: string(item, "provenance"),
    members: item.members.map((member) => {
      const parsed = record(member, "episode member");
      return {
        snapshot_id: string(parsed, "snapshot_id"),
        source_digest: string(parsed, "source_digest"),
      };
    }),
    manual_assessment:
      item.manual_assessment === null
        ? null
        : (() => {
            const assessment = record(
              item.manual_assessment,
              "episode assessment",
            );
            return {
              category: string(assessment, "category"),
              outcome: string(assessment, "outcome"),
              provenance: string(assessment, "provenance"),
              recorded_at: string(assessment, "recorded_at"),
              membership_revision: number(assessment, "membership_revision"),
              members_digest: string(assessment, "members_digest"),
            };
          })(),
  };
}

function parseEpisodeListEntry(value: unknown): EpisodeListEntry {
  const item = record(value, "episode list entry");
  return {
    episode: parseEpisode(item.episode),
    overlapping_episode_ids: strings(
      item.overlapping_episode_ids,
      "episode overlaps",
    ),
  };
}

function parseQuestionCardResult(value: unknown): QuestionCardResult {
  const result = record(value, "question card result");
  const provider = record(result.provider, "question card provider");
  if (!Array.isArray(result.cards))
    throw new Error("Invalid Insights question cards");
  return {
    schema_version: number(result, "schema_version"),
    provider: {
      id: string(provider, "id"),
      version: string(provider, "version"),
      rubric_version: string(provider, "rubric_version"),
      execution_mode: string(provider, "execution_mode"),
      schema_version: number(provider, "schema_version"),
    },
    input_digest: string(result, "input_digest"),
    cards: result.cards.map((value) => {
      const card = record(value, "question card");
      const rows = Array.isArray(card.rows)
        ? card.rows.map((value) => {
            const row = record(value, "question card row");
            const rowValue =
              row.value === null
                ? null
                : (() => {
                    const parsed = record(row.value, "question card value");
                    return {
                      type: string(parsed, "type") as
                        | "count"
                        | "unix_milliseconds"
                        | "milliseconds",
                      value: number(parsed, "value"),
                    };
                  })();
            return {
              id: string(row, "id"),
              unit: string(row, "unit"),
              label: row.label === null ? null : string(row, "label"),
              value: rowValue,
              missing_reason:
                row.missing_reason === null
                  ? null
                  : string(row, "missing_reason"),
            };
          })
        : (() => {
            throw new Error("Invalid Insights question card rows");
          })();
      const coverage = Array.isArray(card.coverage)
        ? card.coverage.map((value) => {
            const item = record(value, "question card coverage");
            return {
              unit: string(item, "unit"),
              observed: number(item, "observed"),
              eligible: number(item, "eligible"),
            };
          })
        : (() => {
            throw new Error("Invalid Insights question card coverage");
          })();
      const denominator =
        card.episode_denominator === null
          ? null
          : (() => {
              const item = record(
                card.episode_denominator,
                "question card denominator",
              );
              return {
                eligible: number(item, "eligible"),
                assessed: number(item, "assessed"),
                unassessed: number(item, "unassessed"),
                explicit_unknown: number(item, "explicit_unknown"),
              };
            })();
      return {
        question: string(card, "question"),
        metric_version: string(card, "metric_version"),
        state: string(card, "state") as "observed" | "partial" | "unavailable",
        rows,
        coverage,
        episode_denominator: denominator,
        evidence_ids: strings(card.evidence_ids, "question card evidence"),
        episode_ids: strings(card.episode_ids, "question card episodes"),
        limitations: strings(card.limitations, "question card limitations"),
        rows_omitted: card.rows_omitted === true,
      };
    }),
  };
}

export async function listEpisodes(): Promise<EpisodeListEntry[]> {
  const response = record(
    await invokeTauri<unknown>("insights_episode_list"),
    "episode list",
  );
  if (!Array.isArray(response.episodes))
    throw new Error("Invalid Insights episode list");
  return response.episodes.map(parseEpisodeListEntry);
}

export async function explainEpisode(id: string): Promise<EpisodeDetail> {
  const response = record(
    await invokeTauri<unknown>("insights_episode_explain", { id }),
    "episode detail",
  );
  const detail = record(response.detail, "episode detail");
  if (!Array.isArray(detail.members) || !Array.isArray(detail.overlap))
    throw new Error("Invalid Insights episode detail");
  return {
    episode: parseEpisode(detail.episode),
    members: detail.members.map(parseInsight),
    overlap: detail.overlap.map((item) => {
      const parsed = record(item, "episode overlap");
      return {
        snapshot_id: string(parsed, "snapshot_id"),
        episode_ids: strings(parsed.episode_ids, "overlap ids"),
      };
    }),
    resolved_at: string(detail, "resolved_at"),
  };
}

export async function createEpisode(snapshotIds: string[]): Promise<Episode> {
  const response = record(
    await invokeTauri<unknown>("insights_episode_create", { snapshotIds }),
    "episode create",
  );
  return parseEpisode(response.episode);
}

export async function annotateEpisode(
  id: string,
  expectedRevision: number,
  category: string,
  outcome: string,
): Promise<Episode> {
  const response = record(
    await invokeTauri<unknown>("insights_episode_annotate", {
      id,
      expectedRevision,
      category,
      outcome,
    }),
    "episode assessment",
  );
  return parseEpisode(response.episode);
}

export async function clearEpisodeAssessment(
  id: string,
  expectedRevision: number,
): Promise<Episode> {
  const response = record(
    await invokeTauri<unknown>("insights_episode_clear_assessment", {
      id,
      expectedRevision,
    }),
    "episode assessment",
  );
  return parseEpisode(response.episode);
}

export async function deleteEpisode(
  id: string,
  expectedRevision: number,
): Promise<void> {
  await invokeTauri("insights_episode_delete", { id, expectedRevision });
}

export async function getQuestionCards(
  snapshotIds: string[],
  episodeIds: string[],
): Promise<QuestionCardResult> {
  const response = record(
    await invokeTauri<unknown>("insights_question_cards", {
      questions: [
        "recorded_activity",
        "episode_outcomes",
        "observed_models",
        "estimated_cost",
      ],
      snapshotIds,
      episodeIds,
    }),
    "question cards",
  );
  return parseQuestionCardResult(response.result);
}
