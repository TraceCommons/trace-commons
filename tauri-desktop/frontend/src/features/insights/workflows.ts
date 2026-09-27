import type { Insight } from "./types";

export type EpisodeMember = { snapshot_id: string; source_digest: string };
export type EpisodeAssessment = {
  category: string;
  outcome: string;
  provenance: string;
  recorded_at: string;
  membership_revision: number;
  members_digest: string;
};
export type Episode = {
  schema_version: number;
  id: string;
  revision: number;
  membership_revision: number;
  created_at: string;
  updated_at: string;
  provenance: string;
  members: EpisodeMember[];
  manual_assessment: EpisodeAssessment | null;
};
export type EpisodeListEntry = {
  episode: Episode;
  overlapping_episode_ids: string[];
};
export type EpisodeOverlap = { snapshot_id: string; episode_ids: string[] };
export type EpisodeDetail = {
  episode: Episode;
  members: Insight[];
  overlap: EpisodeOverlap[];
  resolved_at: string;
};

export type CardValue = {
  type: "count" | "unix_milliseconds" | "milliseconds";
  value: number;
};
export type QuestionCard = {
  question: string;
  metric_version: string;
  state: "observed" | "partial" | "unavailable";
  rows: Array<{
    id: string;
    unit: string;
    label: string | null;
    value: CardValue | null;
    missing_reason: string | null;
  }>;
  coverage: Array<{ unit: string; observed: number; eligible: number }>;
  episode_denominator: {
    eligible: number;
    assessed: number;
    unassessed: number;
    explicit_unknown: number;
  } | null;
  evidence_ids: string[];
  episode_ids: string[];
  limitations: string[];
  rows_omitted: boolean;
};
export type QuestionCardResult = {
  schema_version: number;
  provider: {
    id: string;
    version: string;
    rubric_version: string;
    execution_mode: string;
    schema_version: number;
  };
  input_digest: string;
  cards: QuestionCard[];
};
