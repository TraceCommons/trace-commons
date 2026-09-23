import type { Meta, StoryObj } from "@storybook/react-vite";
import { withQueryClient } from "../../../lib/query/storybook-provider";
import type { Insight } from "../types";
import { InsightEvidencePanel } from "./insight-evidence-panel";

const insight: Insight = {
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
    metrics: [],
    evidence: [{ id: "source-1", source_digest: "b".repeat(64) }],
  },
  estimated_cost_usd: null,
  cost_unavailable_reason: "pricing_unavailable",
  manual_annotation: null,
  outcome_links: [
    {
      id: "c".repeat(64),
      source_digest: "b".repeat(64),
      linked_at: "2026-09-15T10:05:00Z",
      provenance: "user_linked",
      evidence: {
        type: "git_commit",
        evidence: {
          repository_path_digest: "d".repeat(64),
          object_id: "e".repeat(40),
          tree_id: "f".repeat(40),
          parent_ids: [],
          inspected_at: "2026-09-15T10:04:00Z",
          provenance: "inspected_local_object",
        },
      },
    },
  ],
};

const meta = {
  title: "Features/Insights/Evidence",
  component: InsightEvidencePanel,
  decorators: [withQueryClient],
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof InsightEvidencePanel>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Saved: Story = {
  args: {
    insight,
    saved: true,
    busy: false,
    onLinkTestReport: () => {},
    onLinkGit: async () => true,
    onUnlink: () => {},
  },
};

export const Unsaved: Story = {
  args: { ...Saved.args, saved: false },
};
