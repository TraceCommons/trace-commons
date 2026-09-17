import type { Meta, StoryObj } from "@storybook/react-vite";
import { WitnessPanel } from "./witness-panel";

const meta = {
  title: "Features/Settings/WitnessPanel",
  component: WitnessPanel,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof WitnessPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Pinned: Story = {
  args: {
    data: {
      state: "pinned",
      state_code: 1,
      refusal: null,
      url: "https://witness.example",
      signing_address: "0xabc",
      pinned_measurement_count: 1,
      pinned_measurement_line: "One measurement is pinned.",
      pinned_measurements: ["mrtd=aaaaaaaa"],
    },
    state: "ready",
    error: null,
    onRefresh: async () => {},
    onConfigure: async () => {},
    onClear: async () => {},
  },
};
export const LocalOnly: Story = {
  args: {
    data: {
      state: "absent",
      state_code: 0,
      refusal: null,
      url: null,
      signing_address: null,
      pinned_measurement_count: 0,
      pinned_measurements: [],
    },
    state: "ready",
    error: null,
    onRefresh: async () => {},
    onConfigure: async () => {},
    onClear: async () => {},
  },
};
