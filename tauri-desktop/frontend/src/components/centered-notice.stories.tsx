import type { Meta, StoryObj } from "@storybook/react-vite";
import { CenteredNotice } from "./centered-notice";

const meta = {
  title: "Components/CenteredNotice",
  component: CenteredNotice,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof CenteredNotice>;
export default meta;
type Story = StoryObj<typeof meta>;

export const EmptyQueue: Story = {
  args: {
    title: "Nothing is waiting.",
    body: "When a session finishes and goes quiet, it shows up here. Nothing is sent unless you say so.",
  },
};
export const WatcherStopped: Story = {
  args: {
    title: "The watcher isn't running.",
    body: "It didn't answer. Nothing is being noticed or sent while it's stopped, and sessions already waiting stay on this machine.",
  },
};
export const PreviewChanged: Story = {
  args: {
    title: "This one can't be shown.",
    body: "The session file changed while it was being read. Nothing has been sent, and nothing will be until it can be shown to you.",
    tone: "error",
  },
};
