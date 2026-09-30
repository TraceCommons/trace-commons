import type { Meta, StoryObj } from "@storybook/react-vite";
import { ComparisonSpecificationsPanel } from "./comparison-specifications-panel";

const meta = {
  title: "Features/Insights/ComparisonSpecifications",
  component: ComparisonSpecificationsPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ComparisonSpecificationsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const specifications = {
  specifications: [],
  preview: null,
  result: null,
  state: "ready" as const,
  error: null,
  calculate: async () => {},
  save: async () => {},
  evaluate: async () => {},
};

export const Empty: Story = { args: { tasks: [], specifications } };
