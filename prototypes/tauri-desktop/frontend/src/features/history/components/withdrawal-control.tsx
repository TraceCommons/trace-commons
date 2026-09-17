import { Button } from "@/components/ui/button";
import type { HistoryRecord, WithdrawalResult } from "../types";

const reachCopy: Record<
  NonNullable<WithdrawalResult["distribution_reach"]>,
  string
> = {
  not_distributed: "Content deleted. No one outside privacy review saw it.",
  commons_not_distributed: "Content deleted and excluded from future exports.",
  commons_distributed:
    "Content deleted from managed stores. Previously distributed copies cannot be recalled.",
};

function resultCopy(result: WithdrawalResult) {
  return result.distribution_reach
    ? reachCopy[result.distribution_reach]
    : "Withdrawal completed. Distribution reach is unavailable.";
}

export function WithdrawalControl({
  record,
  confirming,
  busy,
  result,
  error,
  onRequest,
  onConfirm,
  onCancel,
}: {
  record: HistoryRecord;
  confirming: boolean;
  busy: boolean;
  result?: WithdrawalResult;
  error?: string;
  onRequest: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  if (result)
    return (
      <div className="col-span-full flex flex-wrap items-center gap-2 border-t border-border py-2.5 text-[11px] leading-[1.45] text-muted-foreground">
        <strong>Withdrawn by you</strong>
        <span>{resultCopy(result)}</span>
        {result.token_deletion_note && (
          <span>{result.token_deletion_note}</span>
        )}
      </div>
    );
  if (record.status === "withdrawn")
    return (
      <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start bg-muted text-muted-foreground">
        Withdrawn by you
      </span>
    );
  if (error)
    return (
      <div className="col-span-full flex flex-wrap items-center gap-2 border-t border-border py-2.5 text-[11px] leading-[1.45] text-muted-foreground text-destructive">
        <span>{error}</span>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={onRequest}
        >
          Try again
        </Button>
      </div>
    );
  if (confirming)
    return (
      <div className="col-span-full flex flex-wrap items-center justify-end gap-2 py-2.5 text-[11px] leading-[1.45] text-muted-foreground">
        <span>
          Withdraw this trace? Content deletion reach is reported after account
          confirmation.
        </span>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={onConfirm}
          disabled={busy}
        >
          {busy ? "Withdrawing…" : "Confirm withdrawal"}
        </Button>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={onCancel}
          disabled={busy}
        >
          Keep it
        </Button>
      </div>
    );
  return (
    <Button
      className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
      type="button"
      onClick={onRequest}
    >
      Withdraw
    </Button>
  );
}
