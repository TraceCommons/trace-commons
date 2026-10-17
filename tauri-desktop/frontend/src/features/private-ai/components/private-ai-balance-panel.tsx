import type { usePrivateAi } from "../hooks/use-private-ai";
import { TertiaryLink } from "@/design-system";

type PrivateAiController = ReturnType<typeof usePrivateAi>;

export function PrivateAiBalancePanel({
  privateAi,
}: {
  privateAi: PrivateAiController;
}) {
  const balance = privateAi.balance?.view;
  return (
    <section className="tc-card mb-2.5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            ACCOUNT BALANCE
          </span>
          <h2>NEAR AI usage</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void privateAi.refreshBalance()}
          disabled={privateAi.busy}
        >
          Refresh balance
        </TertiaryLink>
      </div>
      {balance ? (
        <div className="mt-5 grid gap-[7px] text-[12px] text-tc-secondary">
          <strong>{balance.state_line || "Balance read"}</strong>
          {balance.remaining_line && <span>{balance.remaining_line}</span>}
          {balance.limit_line && <span>{balance.limit_line}</span>}
          {balance.spent_line && <span>{balance.spent_line}</span>}
        </div>
      ) : (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Balance not read. Refresh only when you need the account figure.
        </p>
      )}
      <p className="m-0 tc-caption tc-text-tertiary">
        Account balance requires retained session authority. It is separate from
        inference-key presence.
      </p>
    </section>
  );
}
