import type { Meta, StoryObj } from "@storybook/react-vite";
import { CreditRecordPanel } from "./credit-record-panel";

const meta = {
  title: "History/Credit record",
  component: CreditRecordPanel,
} satisfies Meta<typeof CreditRecordPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Synced: Story = {
  args: {
    finalPoints: 12.4,
    pendingPoints: 3.1,
    refreshedAt: "2026-09-15T10:00:00Z",
  },
};
export const NotSynced: Story = {
  args: { finalPoints: 0, pendingPoints: 0, refreshedAt: null },
};
