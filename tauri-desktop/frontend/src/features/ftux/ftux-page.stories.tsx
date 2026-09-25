import type { Meta, StoryObj } from "@storybook/react-vite";
import { MOCK_STORED_PASSKEY } from "./api/ftux-mock-data";
import { PasskeyFlow, WelcomeBack } from "./components/passkey-flow";
import { FtuxPage } from "./ftux-page";
import "./ftux.css";

const meta = {
  title: "Features/FTUX/FtuxPage",
  component: FtuxPage,
  parameters: { layout: "fullscreen" },
  args: { onComplete: () => {} },
} satisfies Meta<typeof FtuxPage>;
export default meta;
type Story = StoryObj<typeof meta>;

export const ConnectAndForgetJoin: Story = {};
export const ConnectAndForgetFolders: Story = {
  args: { initialScreen: "folders" },
};
export const ConnectAndForgetUses: Story = {
  args: { initialScreen: "uses" },
};
export const CustomizeTools: Story = {
  args: { initialPath: "customize", initialScreen: "tools" },
};
export const CustomizeRules: Story = {
  args: { initialPath: "customize", initialScreen: "rules" },
};
export const CustomizeUses: Story = {
  args: { initialPath: "customize", initialScreen: "uses" },
};
export const ReturningUser: Story = {
  args: { returningPasskey: MOCK_STORED_PASSKEY.name },
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
