import type { Meta, StoryObj } from "@storybook/react-vite";
import { CertificatePanel } from "./certificate-panel";

const meta = {
  title: "Features/Waiting/CertificatePanel",
  component: CertificatePanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof CertificatePanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Held: Story = {
  args: {
    entries: [
      {
        entry_id: "entry-1",
        project_id: "p1",
        project_label: "trace-commons",
        project_path: "~/Documents/trace-commons",
        source: "codex",
        state: "pending",
        size_bytes: 1000,
        discovered_at: "2026-09-15T10:00:00Z",
        subagent_count: 0,
        subagents_dropped: 0,
        holds_certificate: true,
      },
    ],
  },
};
