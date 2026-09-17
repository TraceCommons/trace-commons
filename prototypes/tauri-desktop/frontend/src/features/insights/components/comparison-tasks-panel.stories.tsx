import type { Meta, StoryObj } from "@storybook/react-vite";
import { ComparisonTasksPanel } from "./comparison-tasks-panel";

const meta = {
  title: "Features/Insights/ComparisonTasks",
  component: ComparisonTasksPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ComparisonTasksPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const comparison = {
  tasks: [],
  detail: null,
  state: "ready" as const,
  error: null,
  open: () => {},
  close: () => {},
  create: async () => true,
  replaceEpisodes: async () => true,
  setContext: async () => true,
  setOutcome: async () => true,
  clearOutcome: async () => true,
  reconfirm: async () => true,
};

export const Empty: Story = { args: { episodes: [], comparison } };
