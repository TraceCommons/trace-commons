import type { Meta, StoryObj } from "@storybook/react-vite";
import { CommunityPanel } from "./community-panel";

const meta = {
  title: "Features/History/CommunityPanel",
  component: CommunityPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof CommunityPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Published: Story = {
  args: {
    standing: {
      rank: 12,
      novelty_credit: 84.5,
      accepted_in_window: 19,
      accept_rate: 0.86,
      window_label: "7d",
      public_since: "2026-09-01T10:00:00Z",
      snapshot_at: "2026-09-15T10:00:00Z",
      analytics_withheld: false,
    },
  },
};
export const Withheld: Story = {
  args: {
    standing: {
      rank: null,
      novelty_credit: 0,
      accepted_in_window: 0,
      accept_rate: null,
      window_label: "7d",
      public_since: null,
      snapshot_at: "2026-09-15T10:00:00Z",
      analytics_withheld: true,
    },
  },
};
