import type { Meta, StoryObj } from "@storybook/react-vite";
import { BehaviorSettingsPanel } from "./behavior-settings-panel";

const meta = {
  title: "Features/Settings/BehaviorSettingsPanel",
  component: BehaviorSettingsPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof BehaviorSettingsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const settings = {
  quiescence_secs: 300,
  approval_hold_secs: 30,
  digest_interval_secs: 21600,
};
export const Ready: Story = {
  args: {
    settings,
    busy: null,
    error: null,
    onRefresh: async () => {},
    onSave: async () => {},
  },
};
export const Saving: Story = {
  args: {
    settings,
    busy: "approval_hold",
    error: null,
    onRefresh: async () => {},
    onSave: async () => {},
  },
};
