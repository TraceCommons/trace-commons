import type { Meta, StoryObj } from "@storybook/react-vite";
import { UndoBar } from "./undo-bar";

const meta = {
  title: "Features/Waiting/UndoBar",
  component: UndoBar,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof UndoBar>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Active: Story = {
  args: {
    scope: {
      kind: "entry",
      id: "entry-1",
      hold_until: "2026-09-15T10:00:30Z",
      label: "trace-commons",
    },
    seconds: 19,
    busy: false,
    error: null,
    onUndo: () => undefined,
    onDismiss: () => undefined,
  },
};
