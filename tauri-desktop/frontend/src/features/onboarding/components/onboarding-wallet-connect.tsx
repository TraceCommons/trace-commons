import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { useOnboardingWallet } from "../hooks/use-onboarding-wallet";
import { ButtonPrimary, GlassButton, Input } from "@/design-system";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: This component renders the Rust-owned wallet lifecycle states and controls.
export function OnboardingWalletConnect({
  wallet,
  blocked = false,
}: {
  wallet: ReturnType<typeof useOnboardingWallet>;
  blocked?: boolean;
}) {
  const flow = wallet.flow;
  const disclosure = useContributorDisclosureCopy().data?.wallet;
  if (!flow || flow.state === "Unsupported") return null;

  return (
    <section className="grid gap-4 border-t border-tc-hairline pt-5">
      <div>
        <span className="mb-1.5 block tc-eyebrow">
          NEAR WALLET
        </span>
        <h3>{disclosure?.heading ?? "NEAR wallet signup"}</h3>
        <p className="m-0 tc-label font-normal leading-[17px] tc-text-secondary">
          {disclosure?.disclosure ??
            "Loading wallet connection disclosure…"}
        </p>
      </div>
      <div className="flex flex-col gap-4">
        <div className="tc-field">
          <label className="tc-label" htmlFor="wallet-commons">
            {disclosure?.commons ?? "Commons URL"}
          </label>
          <Input
            id="wallet-commons"
            value={wallet.commons}
            onChange={(event) => wallet.setCommons(event.target.value)}
            placeholder="https://commons.example"
            disabled={wallet.pending || blocked || !flow.can_edit}
          />
        </div>
        {flow.can_start && (
          <div className="tc-field">
            <label className="tc-label" htmlFor="wallet-account">
              {disclosure?.account ?? "NEAR account"}
            </label>
            <Input
              id="wallet-account"
              value={wallet.account}
              onChange={(event) => wallet.setAccount(event.target.value)}
              placeholder="you.near"
              disabled={wallet.pending || blocked}
            />
          </div>
        )}
      </div>
      <div className="flex flex-wrap gap-2.5">
        {flow.can_check && (
          <GlassButton
            type="button"
            onClick={() => void wallet.run("check")}
            disabled={
              wallet.pending || blocked || !wallet.commons.trim() || !disclosure
            }
          >
            Check wallet support
          </GlassButton>
        )}
        {flow.can_start && (
          <ButtonPrimary size="sm"
            type="button"
            onClick={() => void wallet.run("start")}
            disabled={
              wallet.pending ||
              blocked ||
              !wallet.commons.trim() ||
              !wallet.account.trim() ||
              !disclosure
            }
          >
            {wallet.pending ? "Opening…" : "Start signup"}
          </ButtonPrimary>
        )}
        {flow.can_cancel && (
          <GlassButton
            type="button"
            onClick={() => void wallet.run("cancel")}
            disabled={wallet.pending || blocked}
          >
            Cancel
          </GlassButton>
        )}
      </div>
      {wallet.pending && (
        <p className="m-0 tc-label font-normal tc-text-secondary">
          Waiting for wallet ceremony…
        </p>
      )}
      {flow.tone === "refused" && (
        <p className="tc-alert m-0">
          {flow.glyph} {flow.message}
        </p>
      )}
      {flow.tone !== "refused" && flow.message && (
        <p className="m-0 tc-label font-normal tc-text-secondary">{flow.message}</p>
      )}
      {wallet.error && (
        <p className="tc-alert m-0">
          {wallet.error}
        </p>
      )}
    </section>
  );
}
