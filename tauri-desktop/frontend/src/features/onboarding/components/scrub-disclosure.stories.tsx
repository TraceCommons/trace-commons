import type { Meta, StoryObj } from "@storybook/react-vite";
import { ScrubDisclosure } from "./scrub-disclosure";

const meta = {
  title: "Features/Onboarding/ScrubDisclosure",
  component: ScrubDisclosure,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ScrubDisclosure>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  args: {
    open: true,
    names: ["github_token", "aws_access_key", "private_key"],
    state: "ready",
    error: null,
    onRetry: () => {},
    onClose: () => {},
  },
};
export const Loading: Story = {
  args: { ...Loaded.args, state: "loading", names: [] },
};
