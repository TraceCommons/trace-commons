import type { Meta, StoryObj } from "@storybook/react-vite";
import { WaitingProjectFolder } from "./waiting-project-folder";

const meta = {
  title: "Features/Waiting/WaitingProjectFolder",
  component: WaitingProjectFolder,
} satisfies Meta<typeof WaitingProjectFolder>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Ready: Story = {
  args: {
    projectId: "project-1",
    label: "trace-commons",
    path: "~/Documents/trace-commons",
    count: 3,
    busy: false,
    onOpen: () => undefined,
    onSubmitAll: () => undefined,
  },
};
