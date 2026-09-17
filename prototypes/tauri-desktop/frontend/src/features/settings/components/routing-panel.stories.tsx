import type { Meta, StoryObj } from "@storybook/react-vite";
import { RoutingPanel } from "./routing-panel";

const meta = {
  title: "Features/Settings/RoutingPanel",
  component: RoutingPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof RoutingPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
const snapshot = {
  ironwire: {
    mode: "watch",
    port: 8463,
    token_dir: "/Users/example/.ironwire",
  },
};
export const Connected: Story = {
  args: {
    snapshot,
    status: {
      prototype: true,
      state_dir: "preview",
      daemon: {
        schema_version: "1",
        logged_in: true,
        tenant_id: "tenant",
        consent_scopes: [],
        paused: false,
        queue_depth: 0,
        routing: {
          state: "rows_seen",
          derived: false,
          last_refresh_at: "2026-09-15T10:00:00Z",
          unreadable_rows: 0,
        },
        health: { last_error_label: null, since: null },
      },
    },
    discovery: {
      found: true,
      port: 8463,
      token_path: "/Users/example/.ironwire/control.token",
    },
    evidence: {
      outcome: "reachable",
      tools: [
        { id: "codex", installed: true, wired: true },
        { id: "claude", installed: true, wired: false },
      ],
    },
    state: "ready",
    error: null,
    onRefresh: async () => {},
    onCheck: async () => {},
    onConfigure: async () => {},
  },
};
export const Undeclared: Story = {
  args: {
    snapshot: {},
    status: null,
    discovery: { found: false, port: null, token_path: null },
    evidence: null,
    state: "ready",
    error: null,
    onRefresh: async () => {},
    onCheck: async () => {},
    onConfigure: async () => {},
  },
};
