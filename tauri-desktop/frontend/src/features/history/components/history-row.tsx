import { Button } from "@/components/ui/button";
import { isTauriRuntime } from "../../../lib/tauri/core-api";
import {
  canWithdrawStatus,
  historyStatusLabel,
  type SharedHistoryStatusCopy,
} from "../withdrawal-eligibility";
import {
  contributorFacingExplanations,
  historyCreditLine,
} from "../history-view-model";
import type { HistoryRecord, WithdrawalResult } from "../types";
import { AccountSignInControl } from "./account-sign-in-control";
import { WithdrawalControl } from "./withdrawal-control";

export function HistoryRow({
  record,
  accountSignedIn,
  accountStatusPending,
  accountSignInPending,
  accountSignInUrlAvailable,
  accountSignInError,
  openingSignInUrl,
  onAccountSignIn,
  onOpenSignInUrl,
  heldRowFallback,
  statusCopy,
  onOpen,
  confirming,
  busy,
  result,
  error,
  onWithdrawRequest,
  onWithdrawConfirm,
  onWithdrawCancel,
}: {
  record: HistoryRecord;
  accountSignedIn?: boolean;
  accountStatusPending?: boolean;
  accountSignInPending?: boolean;
  accountSignInUrlAvailable?: boolean;
  accountSignInError?: string | null;
  openingSignInUrl?: boolean;
  onAccountSignIn?: () => void;
  onOpenSignInUrl?: () => void;
  heldRowFallback?: string | null;
  statusCopy?: SharedHistoryStatusCopy | null;
  onOpen?: () => void;
  confirming?: boolean;
  busy?: boolean;
  result?: WithdrawalResult;
  error?: string;
  onWithdrawRequest?: () => void;
  onWithdrawConfirm?: () => void;
  onWithdrawCancel?: () => void;
}) {
  const date = new Date(record.submitted_at).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  const status = historyStatusLabel(record.status, statusCopy);
  const canWithdraw = canWithdrawStatus(record.status);
  const creditLine = historyCreditLine(
    record.credit_points_final,
    record.credit_points_pending,
  );
  const visibleExplanations = contributorFacingExplanations(record.explanations);
  const rowExplanations =
    record.status === "quarantined" && visibleExplanations.length === 0
      ? heldRowFallback
        ? [heldRowFallback]
        : []
      : visibleExplanations;
  return (
    <article className="grid grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-b border-border py-3.5">
      <div className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground bg-blue">
        {record.project_label.slice(0, 1).toUpperCase()}
      </div>
      <div className="grid min-w-0 gap-1">
        <strong>{record.project_label}</strong>
        <span>
          {record.source} · {date}
        </span>
        <small>Status: {status}</small>
        {rowExplanations.map((explanation) => (
          <small className="text-muted-foreground" key={explanation}>
            {explanation}
          </small>
        ))}
        {creditLine && <small>{creditLine}</small>}
      </div>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onOpen}
          disabled={!onOpen}
        >
          Open
        </Button>
        {canWithdraw && accountSignedIn === true && onWithdrawRequest && onWithdrawConfirm && onWithdrawCancel && (
          <WithdrawalControl
            record={record}
            confirming={Boolean(confirming)}
            busy={Boolean(busy)}
            result={result}
            error={error}
            onRequest={onWithdrawRequest}
            onConfirm={onWithdrawConfirm}
            onCancel={onWithdrawCancel}
          />
        )}
        {canWithdraw && accountSignedIn !== true && (
          <AccountSignInControl
            checking={Boolean(accountStatusPending)}
            pending={Boolean(accountSignInPending)}
            openingFallback={Boolean(openingSignInUrl)}
            canSignIn={isTauriRuntime()}
            signInUrlAvailable={Boolean(accountSignInUrlAvailable)}
            error={accountSignInError ?? null}
            onSignIn={onAccountSignIn}
            onOpenFallback={onOpenSignInUrl}
          />
        )}
      </div>
    </article>
  );
}
