import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useWithdrawalConfirmationPrompt } from "../../../lib/tauri/use-contributor-copy";
import { canWithdrawStatus } from "../withdrawal-eligibility";
import type { HistoryRecord, WithdrawalResult } from "../types";
import { GlassButton, TertiaryLink } from "@/design-system";

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
  const confirmation = useWithdrawalConfirmationPrompt(confirming);
  if (record.status !== "withdrawn" && !canWithdrawStatus(record.status))
    return null;
  if (confirming)
    return (
      <ResponsiveOverlay
        open
        onOpenChange={(open) => {
          if (!open) onCancel();
        }}
        title="Confirm withdrawal"
        description="Review what withdrawal changes before continuing."
        footer={
          <div className="flex justify-end gap-2">
            <GlassButton
              type="button"
              onClick={onCancel}
              disabled={busy}
            >
              Keep it
            </GlassButton>
            <GlassButton className="tc-text-outside"
              type="button"
              onClick={onConfirm}
              disabled={busy || !confirmation.data}
            >
              {busy ? "Withdrawing…" : "Confirm withdrawal"}
            </GlassButton>
          </div>
        }
      >
        {confirmation.data ? (
          <p className="whitespace-pre-line text-[12px] leading-[1.55] text-tc-secondary">
            {confirmation.data}
          </p>
        ) : confirmation.isError ? (
          <p className="text-[12px] text-tc-outside">
            Withdrawal disclosure unavailable. Withdrawal is disabled.
          </p>
        ) : (
          <p className="text-[12px] text-tc-secondary">
            Loading withdrawal disclosure…
          </p>
        )}
      </ResponsiveOverlay>
    );
  if (result)
    return (
      <div className="col-span-full flex flex-wrap items-center gap-2 border-t border-tc-hairline py-2.5 text-[11px] leading-[1.45] text-tc-secondary">
        <strong>Withdrawn by you</strong>
        <span>{resultCopy(result)}</span>
        {result.token_deletion_note && (
          <span>{result.token_deletion_note}</span>
        )}
      </div>
    );
  if (record.status === "withdrawn")
    return (
      <span className="tc-chip self-start">
        Withdrawn by you
      </span>
    );
  if (error)
    return (
      <div className="col-span-full flex flex-wrap items-center gap-2 border-t border-tc-hairline py-2.5 text-[11px] leading-[1.45] text-tc-secondary text-tc-outside">
        <span>{error}</span>
        <TertiaryLink
          type="button"
          onClick={onRequest}
        >
          Try again
        </TertiaryLink>
      </div>
    );
  return (
    <GlassButton
      className="tc-text-outside"
      type="button"
      onClick={onRequest}
    >
      Withdraw
    </GlassButton>
  );
}
