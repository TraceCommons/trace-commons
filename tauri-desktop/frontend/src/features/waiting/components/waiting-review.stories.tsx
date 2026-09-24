import type { Meta, StoryObj } from "@storybook/react-vite";
import { WaitingReview } from "./waiting-review";

const preview = {
  would_send_bytes: 4096,
  raw_session_bytes: 16384,
  event_count: 8,
  opening_prompt: "Review the local session before contribution.",
  redactions: { local_path: 4, "secret:api_key": 1 },
  redactions_distinct: { local_path: 2, "secret:api_key": 1 },
  pii_labels_present: ["local_path"],
  consent_scopes: ["trace_submission"],
  residual_risk:
    "Some semantic context may remain after deterministic scrubbing.",
  gate_statement:
    "Pattern-based scrubbing may have missed something in the text. Nothing here checks that you read it.",
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

const outcomeCopy = {
  verdict_question: "Did this session do what you asked?",
  worked: "Worked",
  partly: "Partly",
  failed: "Failed",
  verdict_caption: "Optional. This is recorded as the trace outcome; the preview above does not show it.",
  correction_question: "What did it get wrong?",
  correction_placeholder: "Optional",
  correction_caption: "Stored exactly as you write it. This correction is not scrubbed.",
  correction_credential_headline: "Nothing was sent. Your correction looks like it contains a credential.",
  correction_credential_body: "Remove credential, rotate it, and submit again.",
  submit_all_as: "Submit all as...",
  submit_all_as_tooltip: "Record the same outcome for every session in this group.",
  max_correction_chars: 2000,
};

export const Ready: Story = {
  args: {
    preview,
    state: "ready",
    error: null,
    eligibilityCopy: null,
    eligibilityPending: false,
    eligibilityError: false,
    outcomeCopy,
    outcomeCopyPending: false,
    outcomeCopyError: false,
    verdict: null,
    correction: "",
    credentialRefusal: false,
    onVerdictChange: () => undefined,
    onCorrectionChange: () => undefined,
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
