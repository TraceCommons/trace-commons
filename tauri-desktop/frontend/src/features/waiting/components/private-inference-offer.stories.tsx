import type { Meta, StoryObj } from "@storybook/react-vite";
import { PrivateInferenceOffer } from "./private-inference-offer";

const meta = {
  title: "Features/Waiting/PrivateInferenceOffer",
  component: PrivateInferenceOffer,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof PrivateInferenceOffer>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Suggested: Story = {
  args: { offered: true, busy: false, error: null, onAnswer: () => undefined },
};
