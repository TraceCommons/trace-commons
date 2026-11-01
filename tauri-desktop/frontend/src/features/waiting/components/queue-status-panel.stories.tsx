import type { Meta, StoryObj } from "@storybook/react-vite";
import { QueueStatusPanel } from "./queue-status-panel";

const meta = {
  title: "Features/Waiting/QueueStatusPanel",
  component: QueueStatusPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof QueueStatusPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Safeguards: Story = {
  args: {
    health: { last_error_label: null, since: null },
    routing: {
      state: "rows_seen",
      derived: false,
      last_refresh_at: "2026-09-15T10:00:00Z",
      unreadable_rows: 0,
    },
  },
};
export const Blocked: Story = {
  args: {
    health: { last_error_label: "queue-health", since: "2026-09-15T10:00:00Z" },
    routing: {
      state: "token_unreadable",
      derived: false,
      last_refresh_at: null,
      unreadable_rows: 0,
    },
  },
};
