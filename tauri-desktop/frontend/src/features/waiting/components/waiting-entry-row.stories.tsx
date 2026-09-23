import type { Meta, StoryObj } from "@storybook/react-vite";
import { WaitingEntryRow } from "./waiting-entry-row";

const meta = {
  title: "Features/Waiting/WaitingEntryRow",
  component: WaitingEntryRow,
} satisfies Meta<typeof WaitingEntryRow>;
export default meta;
type Story = StoryObj<typeof meta>;

const readyEntry = {
  entry_id: "entry-1",
  project_id: "project-1",
  project_label: "trace-commons",
  project_path: "~/Documents/trace-commons",
  source: "codex",
  state: "pending" as const,
  size_bytes: 8192,
  discovered_at: "2026-09-15T10:00:00Z",
  subagent_count: 2,
  subagents_dropped: 0,
  holds_certificate: true,
  attestation: "attested" as const,
  attestation_copy: {
    state_line:
      "This session carries a checkable copy of the model call it came from.",
    reason_line: null,
    tone: "clear" as const,
  },
};

export const Ready: Story = {
  args: {
    entry: readyEntry,
  },
};

export const Trimmed: Story = {
  args: {
    entry: { ...readyEntry, entry_id: "entry-2", subagents_dropped: 1 },
  },
};
