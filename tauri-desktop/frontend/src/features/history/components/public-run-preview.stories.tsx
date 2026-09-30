import type { Meta, StoryObj } from "@storybook/react-vite";
import type { HistoryDetail, PublicRunDraft } from "../types";
import { PublicRunPreview } from "./public-run-preview";

const detail: HistoryDetail = {
  content_unavailable: false,
  task: "Repair a stalled upload",
  task_success: "success",
  contribution_status: "accepted",
  permitted_uses: [],
  human_correction: null,
  evidence: [],
  contributed_version: "trace.contribution.v1",
  consent_policy_version: "consent.v1",
  redaction_pipeline_version: "redaction.v1",
  publication_version: 0,
  retained_source_slug: null,
  publication: null,
};
const draft: PublicRunDraft = {
  title: "Repair a stalled upload",
  outcome_summary: "The upload completed after the client renewed its session.",
  correction_excerpt: null,
  workflow: "Inspect the response status, renew the session, then retry once.",
  reuse_permission: "cc_by_4_0",
  evidence: [
    { event_id: "event-1", excerpt: "The retry returned a successful status." },
  ],
  source_slug: null,
};
const meta = {
  title: "Features/History/PublicRunPreview",
  component: PublicRunPreview,
} satisfies Meta<typeof PublicRunPreview>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Review: Story = {
  args: {
    detail,
    draft,
    working: false,
    onEdit: () => undefined,
    onPublish: () => undefined,
  },
};
