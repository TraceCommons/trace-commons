import type { Meta, StoryObj } from "@storybook/react-vite";
import type { Insight } from "../types";
import type { EpisodeListEntry, QuestionCardResult } from "../workflows";
import { InsightsWorkflowsPanel } from "./insights-workflows-panel";

const snapshot: Insight = {
  id: "a".repeat(64),
  source_format: "codex",
  boundary: "Local source evidence only.",
  analyzed_at: "2026-09-15T10:00:00Z",
  report: {
    schema_version: 1,
    provider: {
      id: "trace-commons-local",
      version: "1",
      rubric_version: "local",
      execution_mode: "local",
    },
    metrics: [
      {
        id: "events",
        value: 12,
        coverage: { observed: 12, total: 12 },
        evidence_ids: ["e1"],
      },
    ],
    evidence: [{ id: "e1", source_digest: "b".repeat(64) }],
  },
  estimated_cost_usd: null,
  cost_unavailable_reason: "pricing_unavailable",
  manual_annotation: null,
};
const episode: EpisodeListEntry = {
  episode: {
    schema_version: 1,
    id: "00000000-0000-4000-8000-000000000001",
    revision: 1,
    membership_revision: 1,
    created_at: "2026-09-15T10:00:00Z",
    updated_at: "2026-09-15T10:00:00Z",
    provenance: "user_selected_whole_snapshots",
    members: [{ snapshot_id: snapshot.id, source_digest: "b".repeat(64) }],
    manual_assessment: null,
  },
  overlapping_episode_ids: [],
};
const cards: QuestionCardResult = {
  schema_version: 1,
  provider: {
    id: "trace-commons-local",
    version: "1",
    rubric_version: "deterministic-question-cards-v1",
    execution_mode: "local",
    schema_version: 1,
  },
  input_digest: "c".repeat(64),
  cards: [
    {
      question: "recorded_activity",
      metric_version: "recorded-activity-v1",
      state: "observed",
      rows: [
        {
          id: "saved_snapshots",
          unit: "saved_snapshots",
          label: "Saved snapshots",
          value: { type: "count", value: 1 },
          missing_reason: null,
        },
      ],
      coverage: [{ unit: "saved_snapshots", observed: 1, eligible: 1 }],
      episode_denominator: null,
      evidence_ids: ["e1"],
      episode_ids: [],
      limitations: [],
      rows_omitted: false,
    },
  ],
};

const meta = {
  title: "Features/Insights/Workflows",
  component: InsightsWorkflowsPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof InsightsWorkflowsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const base = {
  snapshots: [snapshot],
  workflow: {
    episodes: [],
    detail: null,
    cards: null,
    state: "ready" as const,
    open: () => {},
    close: () => {},
    create: async () => true,
    calculateCards: () => {},
    annotate: async () => true,
  },
};

export const Empty: Story = { args: base };
export const WithSavedEpisodeAndCards: Story = {
  args: { ...base, workflow: { ...base.workflow, episodes: [episode], cards } },
};
