import type { Meta, StoryObj } from "@storybook/react-vite";
import { PublicProfileConsent } from "./public-profile-consent";

const meta = {
  title: "Features/Profile/PublicProfileConsent",
  component: PublicProfileConsent,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof PublicProfileConsent>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Unchecked: Story = {
  args: {
    open: true,
    handle: "manian",
    bio: "Ships billing systems by day.",
    busy: false,
    error: null,
    onConfirm: () => {},
    onCancel: () => {},
  },
};
export const ErrorState: Story = {
  args: {
    ...Unchecked.args,
    error:
      "Profile was not published. Check local enrollment and network access.",
  },
};
