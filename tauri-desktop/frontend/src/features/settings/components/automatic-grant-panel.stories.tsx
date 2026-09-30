import type { Meta, StoryObj } from "@storybook/react-vite";
import { AutomaticGrantPanel } from "./automatic-grant-panel";

const meta = {
  title: "Features/Settings/AutomaticGrantPanel",
  component: AutomaticGrantPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof AutomaticGrantPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const actions = {
  onRefresh: () => Promise.resolve(),
  onWithdraw: () => Promise.resolve(),
};

export const Granted: Story = {
  args: {
    grant: {
      granted: true,
      granted_at: "2026-09-26T12:00:00Z",
      on_disk_recorded: true,
    },
    state: "ready",
    error: null,
    withdrawn: false,
    ...actions,
  },
};
export const NotGranted: Story = {
  args: {
    grant: { granted: false, granted_at: null, on_disk_recorded: false },
    state: "ready",
    error: null,
    withdrawn: false,
    ...actions,
  },
};
export const Withdrawn: Story = {
  args: {
    grant: { granted: false, granted_at: null, on_disk_recorded: false },
    state: "ready",
    error: null,
    withdrawn: true,
    ...actions,
  },
};
