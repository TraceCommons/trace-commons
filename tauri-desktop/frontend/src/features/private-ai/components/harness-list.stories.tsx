import type { Meta, StoryObj } from "@storybook/react-vite";
import { HarnessListPanel } from "./harness-list";

const meta = {
  title: "Features/PrivateAI/HarnessListPanel",
  component: HarnessListPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof HarnessListPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const data = {
  harnesses: [
    {
      id: "codex",
      name: "Codex",
      installed: true,
      connected: true,
      config_path: "/Users/example/.codex/config.toml",
      connect_command: "codex",
      state: "connected_no_calls",
      can_connect: false,
      can_disconnect: true,
      state_line: "Connected; no call seen here.",
    },
    {
      id: "claude",
      name: "Claude Code",
      installed: false,
      connected: false,
      config_path: null,
      connect_command: "claude",
      state: "not_connected",
      can_connect: false,
      can_disconnect: false,
      state_line: "Not connected.",
    },
  ],
  view: {
    title: "Tools on this computer",
    what: "Choose tools one at a time.",
    spend_scope: "This amount covers calls answered here since midnight.",
    none_found: "No supported tools found.",
    credential_notice: "Connect requires a private inference credential.",
    spend_line: "Cost of calls answered here since midnight: $0.00.",
    state_lines: {},
  },
};

const actions = {
  plan: null,
  actionState: "idle" as const,
  error: null,
  onPlan: () => Promise.resolve(),
  onCommit: () => Promise.resolve(),
  onCancel: () => {},
};
export const Connected: Story = {
  args: {
    data,
    state: "ready",
    onRefresh: () => Promise.resolve(),
    ...actions,
  },
};
export const Loading: Story = {
  args: {
    data: null,
    state: "loading",
    onRefresh: () => Promise.resolve(),
    ...actions,
  },
};
