import type { Meta, StoryObj } from "@storybook/react-vite";
import { WaitingProjectGroup } from "./waiting-project-group";

const meta = {
  title: "Features/Waiting/WaitingProjectGroup",
  component: WaitingProjectGroup,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof WaitingProjectGroup>;
export default meta;
type Story = StoryObj<typeof meta>;

const entries = [
  {
    entry_id: "entry-1",
    project_id: "project-1",
    project_label: "trace-commons",
    project_path: "~/Documents/trace-commons",
    source: "codex",
    state: "pending",
    size_bytes: 8192,
    discovered_at: "2026-09-15T10:00:00Z",
    subagent_count: 0,
    subagents_dropped: 0,
  },
  {
    entry_id: "entry-2",
    project_id: "project-1",
    project_label: "trace-commons",
    project_path: "~/Documents/trace-commons",
    source: "claude-code",
    state: "pending",
    size_bytes: 16384,
    discovered_at: "2026-09-15T10:02:00Z",
    subagent_count: 1,
    subagents_dropped: 0,
  },
];

export const Ready: Story = {
  args: {
    projectId: "project-1",
    label: "trace-commons",
    entries,
    selectedId: null,
    busy: false,
    onReview: () => undefined,
    onSubmitAll: () => undefined,
  },
};
export const Submitting: Story = { args: { ...Ready.args, busy: true } };
