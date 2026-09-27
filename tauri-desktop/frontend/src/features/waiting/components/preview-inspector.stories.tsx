import type { Meta, StoryObj } from "@storybook/react-vite";
import { withQueryClient } from "../../../lib/query/storybook-provider";
import { PreviewInspector } from "./preview-inspector";

const meta = {
  title: "Features/Waiting/PreviewInspector",
  component: PreviewInspector,
  decorators: [withQueryClient],
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof PreviewInspector>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Closed: Story = {
  args: { preview: null, open: false, onClose: () => undefined },
};
