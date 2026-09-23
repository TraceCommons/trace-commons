import type { Meta, StoryObj } from "@storybook/react-vite";
import { PrivacyControlsPanel } from "./privacy-controls-panel";

const meta = {
  title: "Features/Settings/PrivacyControlsPanel",
  component: PrivacyControlsPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof PrivacyControlsPanel>;
export default meta;
type Story = StoryObj<typeof meta>;

const storage = {
  capture_enabled: false,
  capture_label: "Enable local token capture",
  capture_confirmation:
    "Token capture stores raw request and response data on this device.",
  capture_notice: "Local capture disabled.",
  state_line: "0 bytes of local review data.",
  scope_note:
    "Local cleanup preserves original agent files and server contributions.",
  cleanup_label: "Remove submitted local copies",
  discard_label: "Discard unsubmitted token reviews",
  discard_confirmation: "Discard unsubmitted token reviews?",
  failure_line: "Cleanup could not complete.",
};

export const Disabled: Story = {
  args: {
    settings: {
      ironwire_attested_bodies: false,
      token_distributions_contribution: false,
    },
    storage,
    state: "ready",
    error: null,
    onRefresh: () => Promise.resolve(),
    onInference: () => Promise.resolve(),
    onToken: () => Promise.resolve(),
    onCapture: () => Promise.resolve(),
    onCleanup: () => Promise.resolve(),
  },
};
export const Loading: Story = {
  args: {
    settings: {},
    storage: null,
    state: "loading",
    error: null,
    onRefresh: () => Promise.resolve(),
    onInference: () => Promise.resolve(),
    onToken: () => Promise.resolve(),
    onCapture: () => Promise.resolve(),
    onCleanup: () => Promise.resolve(),
  },
};
