import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { withdrawHistory } from "../api/history-api";
import { historyKeys } from "../api/query-keys";
import type { WithdrawalResult } from "../types";

export function useHistoryWithdrawal() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, WithdrawalResult>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const mutation = useMutation({
    mutationFn: (submissionId: string) => withdrawHistory(submissionId),
    onSuccess: async (result, submissionId) => {
      setResults((current) => ({ ...current, [submissionId]: result }));
      setConfirmingId(null);
      await queryClient.invalidateQueries({
        queryKey: historyKeys.scope(core.scope),
      });
    },
  });
  const request = (submissionId: string) => {
    setErrors((current) => ({ ...current, [submissionId]: "" }));
    setConfirmingId(submissionId);
  };
  const cancel = () => setConfirmingId(null);
  const confirm = async (submissionId: string) => {
    setErrors((current) => ({ ...current, [submissionId]: "" }));
    try {
      await mutation.mutateAsync(submissionId);
    } catch (error) {
      const message =
        error instanceof Error &&
        error.message.includes("account-session-required")
          ? "Account session required. Sign in through the commons before withdrawing."
          : "Withdrawal failed. Nothing is reported as deleted.";
      setErrors((current) => ({ ...current, [submissionId]: message }));
    }
  };
  return {
    confirmingId,
    busyId: mutation.isPending ? (mutation.variables ?? null) : null,
    results,
    errors,
    request,
    cancel,
    confirm,
    mutation,
  };
}
