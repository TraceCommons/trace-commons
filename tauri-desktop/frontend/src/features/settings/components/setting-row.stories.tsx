import type { Meta, StoryObj } from "@storybook/react-vite";
import { SettingRow } from "./setting-row";

const meta = {
  title: "Features/Settings/SettingRow",
  component: SettingRow,
} satisfies Meta<typeof SettingRow>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: {
    label: "Poll interval",
    value: "60 sec",
    detail: "How often local sources are checked",
  },
};
