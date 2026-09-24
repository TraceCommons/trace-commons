import type { Meta, StoryObj } from "@storybook/react-vite";
import { AuditPanel } from "./audit-panel";

const meta = {
  title: "Features/Settings/AuditPanel",
  component: AuditPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof AuditPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const entries = [
  {
    at: "2026-09-15T09:30:00Z",
    action: "armed-auto-upload",
    project_label: "trace-commons",
    detail: null,
  },
  {
    at: "2026-09-14T16:10:00Z",
    action: "consent-scopes-changed",
    project_label: null,
    detail: null,
  },
];

export const Recent: Story = {
  args: {
    entries,
    state: "ready",
    error: null,
    onRefresh: () => Promise.resolve(),
  },
};
export const Empty: Story = {
  args: {
    entries: [],
    state: "ready",
    error: null,
    onRefresh: () => Promise.resolve(),
  },
};
