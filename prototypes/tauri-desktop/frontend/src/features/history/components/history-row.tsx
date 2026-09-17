import { Button } from "@/components/ui/button";
import type { HistoryRecord, WithdrawalResult } from "../types";
import { WithdrawalControl } from "./withdrawal-control";

export function HistoryRow({
  record,
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
  const status = record.status.replaceAll("_", " ");
  return (
    <article className="grid grid-cols-[38px_minmax(0,1fr)_auto_auto] items-center gap-3.5 border-b border-border py-3.5 max-[860px]:grid-cols-[38px_minmax(0,1fr)_auto]">
      <div className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground bg-blue">
        {record.project_label.slice(0, 1).toUpperCase()}
      </div>
      <div className="grid min-w-0 gap-1">
        <strong>{record.project_label}</strong>
        <span>
          {record.source} · {date}
        </span>
      </div>
      <div className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
        <strong>{status}</strong>
        <span>
          {record.credit_points_final ?? record.credit_points_pending} credit
          points
        </span>
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
        {onWithdrawRequest && onWithdrawConfirm && onWithdrawCancel && (
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
      </div>
    </article>
  );
}
