import type { Meta, StoryObj } from "@storybook/react-vite";
import { withQueryClient } from "../../../lib/query/storybook-provider";
import { SourceRootsPanel } from "./source-roots-panel";

const meta = {
  title: "Features/Settings/SourceRootsPanel",
  component: SourceRootsPanel,
  decorators: [withQueryClient],
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof SourceRootsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Unset: Story = {
  args: { snapshot: {}, busy: false, error: null, onSave: async () => {} },
};
export const CodexWatching: Story = {
  args: {
    snapshot: { codex_source_mode: "watch", claude_source_mode: "off" },
    busy: false,
    error: null,
    onSave: async () => {},
  },
};
