import { Button } from "@/components/ui/button";
import type { usePrivateAi } from "../hooks/use-private-ai";

type PrivateAiController = ReturnType<typeof usePrivateAi>;

export function PrivateAiBalancePanel({
  privateAi,
}: {
  privateAi: PrivateAiController;
}) {
  const balance = privateAi.balance?.view;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            ACCOUNT BALANCE
          </span>
          <h2>NEAR AI usage</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void privateAi.refreshBalance()}
          disabled={privateAi.busy}
        >
          Refresh balance
        </Button>
      </div>
      {balance ? (
        <div className="mt-5 grid gap-[7px] text-[12px] text-muted-foreground">
          <strong>{balance.state_line || "Balance read"}</strong>
          {balance.remaining_line && <span>{balance.remaining_line}</span>}
          {balance.limit_line && <span>{balance.limit_line}</span>}
          {balance.spent_line && <span>{balance.spent_line}</span>}
        </div>
      ) : (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Balance not read. Refresh only when you need the account figure.
        </p>
      )}
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Account balance requires retained session authority. It is separate from
        inference-key presence.
      </p>
    </section>
  );
}
