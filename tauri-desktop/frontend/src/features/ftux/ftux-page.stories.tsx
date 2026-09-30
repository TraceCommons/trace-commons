import type { Meta, StoryObj } from "@storybook/react-vite";
import { withQueryClient } from "../../lib/query/storybook-provider";
import { MOCK_STORED_PASSKEY } from "./api/ftux-mock-data";
import { PasskeyFlow, WelcomeBack } from "./components/passkey-flow";
import { FtuxPage } from "./ftux-page";
import "./ftux.css";

const meta = {
  title: "Features/FTUX/FtuxPage",
  component: FtuxPage,
  parameters: { layout: "fullscreen" },
  decorators: [withQueryClient],
  args: { onComplete: () => {} },
} satisfies Meta<typeof FtuxPage>;
export default meta;
type Story = StoryObj<typeof meta>;

export const QuickJoin: Story = {};
export const JoinWithPasskeyCard: Story = { args: { showPasskey: true } };
export const QuickFolders: Story = {
  args: { initialScreen: "folders" },
};
export const QuickUses: Story = {
  args: { initialScreen: "uses" },
};
export const CustomTools: Story = {
  args: { initialPath: "custom", initialScreen: "tools" },
};
export const CustomRules: Story = {
  args: { initialPath: "custom", initialScreen: "rules" },
};
export const CustomUses: Story = {
  args: { initialPath: "custom", initialScreen: "uses" },
};
export const ReturningUser: Story = {
  args: { returningPasskey: MOCK_STORED_PASSKEY },
};

const passkeyStep = (initialStep: "choose" | "sign-in"): Story => ({
  render: () => (
    <div className="ftux">
      <PasskeyFlow
        initialStep={initialStep}
        onDone={() => {}}
        onClose={() => {}}
      />
    </div>
  ),
});

export const PasskeyChoose = passkeyStep("choose");
export const PasskeySystemSignIn = passkeyStep("sign-in");
export const PasskeyWelcomeBack: Story = {
  render: () => (
    <div className="ftux">
      <WelcomeBack
        passkeyName={MOCK_STORED_PASSKEY.name}
        onSignIn={() => {}}
        onOtherOptions={() => {}}
      />
    </div>
  ),
};
