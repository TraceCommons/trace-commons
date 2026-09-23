import type { Meta, StoryObj } from "@storybook/react-vite";
import { ConsentSettingsPanel } from "./consent-settings-panel";

const meta = {
  title: "Features/Settings/ConsentSettingsPanel",
  component: ConsentSettingsPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ConsentSettingsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
const options = [
  {
    name: "debugging_evaluation",
    description: "Required baseline use.",
    always_on: true,
    grants_data_use: true,
  },
  {
    name: "research",
    description: "Allow research use.",
    always_on: false,
    grants_data_use: true,
  },
  {
    name: "public_attribution",
    description: "Allow public credit.",
    always_on: false,
    grants_data_use: false,
  },
];
export const Ready: Story = {
  args: {
    options,
    granted: ["debugging_evaluation", "research"],
    state: "ready",
    error: null,
    onRefresh: async () => {},
    onToggle: async () => {},
  },
};
