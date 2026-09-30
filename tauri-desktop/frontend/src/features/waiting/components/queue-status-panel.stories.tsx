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
    budget: {
      bytes_today: 100,
      max_bytes_per_day: 500 * 1024 * 1024,
      bytes_remaining: 400 * 1024 * 1024,
      uploads_today: 2,
      max_uploads_per_day: 100,
      uploads_remaining: 98,
      blocked: false,
      blocked_entries: 0,
      blocked_bytes: 0,
    },
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
    budget: {
      bytes_today: 500,
      max_bytes_per_day: 500,
      bytes_remaining: 0,
      uploads_today: 100,
      max_uploads_per_day: 100,
      uploads_remaining: 0,
      blocked: true,
      blocked_entries: 3,
      blocked_bytes: 5000,
    },
    routing: {
      state: "token_unreadable",
      derived: false,
      last_refresh_at: null,
      unreadable_rows: 0,
    },
  },
};
