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
    copy: {
      list_title: "Sessions carrying a witness certificate",
      row_line:
        "A witness certificate is held for this session, so what you send carries signed proof of the reviewed bytes.",
      list_empty:
        "Nothing here yet. A session joins this list once your witness has reviewed it.",
    },
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
        attestation: "attested",
        attestation_copy: {
          state_line:
            "This session carries a checkable copy of the model call it came from.",
          reason_line: null,
          tone: "clear",
        },
      },
    ],
  },
};

export const Empty: Story = {
  args: {
    copy: {
      list_title: "Sessions carrying a witness certificate",
      row_line:
        "A witness certificate is held for this session, so what you send carries signed proof of the reviewed bytes.",
      list_empty:
        "Nothing here yet. A session joins this list once your witness has reviewed it.",
    },
    entries: [
      {
        entry_id: "entry-2",
        project_id: "p2",
        project_label: "trace-commons",
        project_path: "~/Documents/trace-commons",
        source: "codex",
        state: "pending",
        size_bytes: 1000,
        discovered_at: "2026-09-15T10:00:00Z",
        subagent_count: 0,
        subagents_dropped: 0,
        holds_certificate: false,
        attestation: "unattested_configuration",
        attestation_copy: {
          state_line:
            "This session carries no copy of the model call it came from. A setting decides whether the ones you record from now on will.",
          reason_line: null,
          tone: "attention",
        },
      },
    ],
  },
};
