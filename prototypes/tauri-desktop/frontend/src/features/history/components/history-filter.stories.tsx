import type { Meta, StoryObj } from "@storybook/react-vite";
import { HistoryFilterBar } from "./history-filter";

const meta = {
  title: "Features/History/HistoryFilterBar",
  component: HistoryFilterBar,
} satisfies Meta<typeof HistoryFilterBar>;
export default meta;
type Story = StoryObj<typeof meta>;

export const AllSelected: Story = {
  args: {
    value: "all",
    counts: {
      all: 12,
      accepted: 7,
      submitted: 2,
      quarantined: 1,
      withdrawn: 2,
    },
    onChange: () => undefined,
  },
};
