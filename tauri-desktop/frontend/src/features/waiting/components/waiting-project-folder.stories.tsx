import type { Meta, StoryObj } from "@storybook/react-vite";
import { WaitingProjectFolder } from "./waiting-project-folder";

const meta = {
  title: "Features/Waiting/WaitingProjectFolder",
  component: WaitingProjectFolder,
} satisfies Meta<typeof WaitingProjectFolder>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Ready: Story = {
  args: {
    projectId: "project-1",
    label: "trace-commons",
    path: "~/Documents/trace-commons",
    count: 3,
    entries: [
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
    ],
    busy: false,
    onOpen: () => undefined,
    onSubmitAll: () => undefined,
    onSubmitAllAs: () => undefined,
    onIgnore: async () => undefined,
  },
};
