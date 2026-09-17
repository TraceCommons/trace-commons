import type { Meta, StoryObj } from "@storybook/react-vite";
import { ProjectsPanel } from "./projects-panel";

const meta = {
  title: "Features/Settings/ProjectsPanel",
  component: ProjectsPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof ProjectsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const projects = [
  {
    project_id: "project-a",
    project_label: "trace-commons",
    project_path: "~/Documents/trace-commons",
    mode: "notify_only" as const,
    configured: true,
    is_unresolved_bucket: false,
    pending_count: 3,
    contributable_count: 3,
  },
  {
    project_id: "unknown",
    project_label: "unknown-project",
    project_path: "",
    mode: "notify_only" as const,
    configured: false,
    is_unresolved_bucket: true,
    pending_count: 1,
  },
];

export const Review: Story = {
  args: {
    projects,
    state: "ready",
    error: null,
    onRefresh: () => Promise.resolve(),
    onSetMode: () => Promise.resolve(),
  },
};
export const Loading: Story = {
  args: {
    projects: [],
    state: "loading",
    error: null,
    onRefresh: () => Promise.resolve(),
    onSetMode: () => Promise.resolve(),
  },
};
