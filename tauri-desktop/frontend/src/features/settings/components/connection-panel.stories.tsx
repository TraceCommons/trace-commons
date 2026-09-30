import type { Meta, StoryObj } from "@storybook/react-vite";
import { ConnectionPanel } from "./connection-panel";

const meta = {
  title: "Features/Settings/ConnectionPanel",
  component: ConnectionPanel,
  parameters: { layout: "centered" },
} satisfies Meta<typeof ConnectionPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const status = {
  state_dir: "storybook",
  startup: "running" as const,
  daemon: {
    schema_version: "v1",
    logged_in: true,
    tenant_id: "tenant-story",
    consent_scopes: ["debugging_evaluation"],
    paused: false,
    queue_depth: 0,
    health: { last_error_label: null, since: null },
  },
};

export const Connected: Story = {
  args: {
    status,
    settings: {
      claude_source_mode: "watch",
      codex_source_mode: "watch",
      gemini_source_mode: "off",
      cline_source_mode: "not_declared",
      opencode_source_mode: "watch",
      near_ai_configured: true,
    },
  },
};
export const LocalOnly: Story = {
  args: {
    status: {
      ...status,
      daemon: { ...status.daemon, logged_in: false, tenant_id: null },
    },
    settings: {},
  },
};
