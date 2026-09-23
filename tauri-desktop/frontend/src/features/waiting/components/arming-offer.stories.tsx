import type { Meta, StoryObj } from "@storybook/react-vite";
import { ArmingOffer } from "./arming-offer";

const meta = {
  title: "Features/Waiting/ArmingOffer",
  component: ArmingOffer,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ArmingOffer>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Suggested: Story = {
  args: {
    offer: {
      project_id: "p1",
      project_label: "trace-commons",
      contributed_count: 12,
    },
    busy: false,
    error: null,
    onAccept: () => undefined,
    onDecline: () => undefined,
  },
};
