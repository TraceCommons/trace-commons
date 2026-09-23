import type { Meta, StoryObj } from "@storybook/react-vite";
import { HistoryRow } from "./history-row";

const meta = {
  title: "Features/History/HistoryRow",
  component: HistoryRow,
} satisfies Meta<typeof HistoryRow>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Accepted: Story = {
  args: {
    record: {
      submission_id: "submission-1",
      project_id: "project-1",
      submitted_at: "2026-09-15T10:00:00Z",
      project_label: "trace-commons",
      source: "codex",
      status: "accepted",
      credit_points_pending: 0,
      credit_points_final: 4,
      explanations: [],
    },
  },
};
