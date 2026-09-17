import type { Meta, StoryObj } from "@storybook/react-vite";
import { WaitingReview } from "./waiting-review";

const preview = {
  would_send_bytes: 4096,
  raw_session_bytes: 16384,
  event_count: 8,
  opening_prompt: "Review the local session before contribution.",
  redactions: { "local path": 4, secret: 1 },
  redactions_distinct: { "local path": 2, secret: 1 },
  pii_labels_present: ["local path"],
  consent_scopes: ["trace_submission"],
  residual_risk:
    "Some semantic context may remain after deterministic scrubbing.",
  input_fingerprint: "sha256:fixture",
  enrolled: true,
  subagent_count: 2,
  subagents_dropped: 0,
  entry: {
    entry_id: "entry-1",
    project_id: "project-1",
    project_label: "trace-commons",
    project_path: "~/Documents/trace-commons",
    source: "codex",
    state: "pending",
    size_bytes: 16384,
    discovered_at: "2026-09-15T10:00:00Z",
    subagent_count: 2,
    subagents_dropped: 0,
  },
};

const meta = {
  title: "Features/Waiting/WaitingReview",
  component: WaitingReview,
  parameters: { layout: "centered" },
} satisfies Meta<typeof WaitingReview>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Ready: Story = {
  args: {
    preview,
    state: "ready",
    error: null,
    onApprove: () => undefined,
    onDismiss: () => undefined,
    onInspect: () => undefined,
  },
};
export const NotEnrolled: Story = {
  args: { ...Ready.args, preview: { ...preview, enrolled: false } },
};
export const Loading: Story = {
  args: { ...Ready.args, preview: null, state: "loading" },
};
