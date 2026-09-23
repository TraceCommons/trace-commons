import { Button } from "@/components/ui/button";
import {
  Field,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { useOnboardingWallet } from "../hooks/use-onboarding-wallet";

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
    <section className="grid gap-4 border-t border-border pt-5">
      <div>
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          NEAR WALLET
        </span>
        <h3>{disclosure?.heading ?? "NEAR wallet signup"}</h3>
        <p className="m-0 text-[12px] leading-[1.55] text-muted-foreground">
          {disclosure?.disclosure ??
            "Loading wallet connection disclosure…"}
        </p>
      </div>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="wallet-commons">
            {disclosure?.commons ?? "Commons URL"}
          </FieldLabel>
          <Input
            id="wallet-commons"
            value={wallet.commons}
            onChange={(event) => wallet.setCommons(event.target.value)}
            placeholder="https://commons.example"
            disabled={wallet.pending || blocked || !flow.can_edit}
          />
        </Field>
        {flow.can_start && (
          <Field>
            <FieldLabel htmlFor="wallet-account">
              {disclosure?.account ?? "NEAR account"}
            </FieldLabel>
            <Input
              id="wallet-account"
              value={wallet.account}
              onChange={(event) => wallet.setAccount(event.target.value)}
              placeholder="you.near"
              disabled={wallet.pending || blocked}
            />
          </Field>
        )}
      </FieldGroup>
      <div className="flex flex-wrap gap-2.5">
        {flow.can_check && (
          <Button
            type="button"
            variant="outline"
            onClick={() => void wallet.run("check")}
            disabled={
              wallet.pending || blocked || !wallet.commons.trim() || !disclosure
            }
          >
            Check wallet support
          </Button>
        )}
        {flow.can_start && (
          <Button
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
          </Button>
        )}
        {flow.can_cancel && (
          <Button
            type="button"
            variant="ghost"
            onClick={() => void wallet.run("cancel")}
            disabled={wallet.pending || blocked}
          >
            Cancel
          </Button>
        )}
      </div>
      {wallet.pending && (
        <p className="m-0 text-[12px] text-muted-foreground">
          Waiting for wallet ceremony…
        </p>
      )}
      {flow.tone === "refused" && (
        <p className="m-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {flow.glyph} {flow.message}
        </p>
      )}
      {flow.tone !== "refused" && flow.message && (
        <p className="m-0 text-[12px] text-muted-foreground">{flow.message}</p>
      )}
      {wallet.error && (
        <p className="m-0 rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {wallet.error}
        </p>
      )}
    </section>
  );
}
