import type { Meta, StoryObj } from "@storybook/react-vite";
import { RedactedTranscript } from "./redacted-transcript";

const body = `[
  {
    "event_type": "user_message",
    "text": "Read <PRIVATE_PATH_1>/billing.toml"
  },
  {
    "event_type": "tool_result",
    "text": "STRIPE_SECRET_KEY=[REDACTED_SECRET_1]"
  }
]`;

const encoder = new TextEncoder();
const firstTurn = body.indexOf("  {");
const secondTurn = body.indexOf("  {", firstTurn + 1);
const meta = {
  title: "Features/Waiting/RedactedTranscript",
  component: RedactedTranscript,
  parameters: { layout: "centered" },
} satisfies Meta<typeof RedactedTranscript>;
export default meta;
type Story = StoryObj<typeof meta>;

export const MarkersStayVisible: Story = {
  args: {
    body,
    turns: [
      {
        index: 0,
        role: "user",
        tool_name: null,
        byte_offset: encoder.encode(body.slice(0, firstTurn)).byteLength,
        byte_len: 80,
      },
      {
        index: 1,
        role: "tool_result",
        tool_name: "bash",
        byte_offset: encoder.encode(body.slice(0, secondTurn)).byteLength,
        byte_len: 90,
      },
    ],
  },
};

export const WithoutTurnIndex: Story = { args: { body, turns: [] } };
