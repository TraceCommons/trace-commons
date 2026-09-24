import type { Meta, StoryObj } from "@storybook/react-vite";
import type { HistoryDetail } from "../types";
import { PublicRunForm } from "./public-run-form";

const detail: HistoryDetail = {
  content_unavailable: false,
  task: "Repair a stalled upload",
  task_success: "success",
  contribution_status: "accepted",
  permitted_uses: ["debugging_evaluation"],
  human_correction: "Renew the account session before retrying.",
  evidence: [
    {
      event_id: "event-1",
      kind: "tool_result",
      excerpt: "The retry returned a successful status.",
    },
  ],
  contributed_version: "trace.contribution.v1",
  consent_policy_version: "consent.v1",
  redaction_pipeline_version: "redaction.v1",
  publication_version: 0,
  retained_source_slug: null,
  publication: null,
};

const meta = {
  title: "Features/History/PublicRunForm",
  component: PublicRunForm,
} satisfies Meta<typeof PublicRunForm>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {
  args: {
    detail,
    working: false,
    error: null,
    editingPublished: false,
    onReview: () => undefined,
    onCancel: () => undefined,
  },
};
