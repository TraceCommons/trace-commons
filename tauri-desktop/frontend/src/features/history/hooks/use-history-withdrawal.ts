import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import {
  getAccountSessionStatus,
  openAccountSignInUrl,
  signInToAccount,
} from "../../../lib/tauri/account-session-api";
import { isTauriRuntime, listenTauri } from "../../../lib/tauri/core-api";
import { coreKeys } from "../../../lib/tauri/query-keys";
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
  const [requiresSignIn, setRequiresSignIn] = useState(false);
  const [signInUrl, setSignInUrl] = useState<string | null>(null);
  const [signInError, setSignInError] = useState<string | null>(null);
  const [openingSignInUrl, setOpeningSignInUrl] = useState(false);
  const accountSession = useQuery({
    queryKey: coreKeys.accountSession,
    queryFn: getAccountSessionStatus,
    enabled: isTauriRuntime(),
  });
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
  const signInMutation = useMutation({
    mutationFn: async () => {
      setSignInUrl(null);
      setSignInError(null);
      const unlisten = await listenTauri("account-sign-in-url", (payload) => {
        setSignInUrl(typeof payload === "string" ? payload : null);
      });
      try {
        return await signInToAccount();
      } finally {
        unlisten();
      }
    },
    onSuccess: async () => {
      setSignInUrl(null);
      void core.refresh().catch(() => undefined);
      try {
        const refreshed = await getAccountSessionStatus();
        queryClient.setQueryData(coreKeys.accountSession, refreshed);
        if (refreshed.signed_in) {
          setRequiresSignIn(false);
          setSignInError(null);
          await queryClient.invalidateQueries({
            queryKey: historyKeys.scope(core.scope),
          });
        } else {
          setRequiresSignIn(true);
          setSignInError(
            "Sign-in finished, but the account session is not active. Withdrawal remains unavailable.",
          );
        }
      } catch {
        setRequiresSignIn(true);
        setSignInError(
          "Sign-in finished, but account status could not be verified. Retry sign-in before withdrawing.",
        );
      }
    },
    onError: () => {
      setSignInUrl(null);
      setSignInError(
        "Sign-in did not finish. Withdrawal was not completed; try signing in again.",
      );
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
      if (
        error instanceof Error &&
        error.message.includes("account-session-required")
      ) {
        setRequiresSignIn(true);
        setSignInError(
          "Commons rejected the stored account session. Nothing was withdrawn. Sign in again to retry.",
        );
        queryClient.setQueryData(coreKeys.accountSession, {
          signed_in: false,
          expires_at: null,
        });
      } else {
        setErrors((current) => ({
          ...current,
          [submissionId]: "Withdrawal failed. Nothing is reported as deleted.",
        }));
      }
    }
  };
  const startSignIn = () => {
    if (!signInMutation.isPending && isTauriRuntime()) {
      signInMutation.mutate();
    }
  };
  const openSignInUrl = async () => {
    if (!signInUrl || openingSignInUrl) return;
    setOpeningSignInUrl(true);
    try {
      await openAccountSignInUrl(signInUrl);
    } catch {
      setSignInError(
        "Could not open the sign-in page. Try the browser window or restart sign-in.",
      );
    } finally {
      setOpeningSignInUrl(false);
    }
  };
  return {
    accountSignedIn:
      !requiresSignIn && accountSession.data?.signed_in === true,
    accountStatusPending: accountSession.isPending,
    accountSignInPending: signInMutation.isPending,
    accountSignInUrl: signInUrl,
    accountSignInError:
      signInError ??
      (accountSession.isError
        ? "Account session could not be checked. Sign in to continue."
        : null),
    openingSignInUrl,
    startSignIn,
    openSignInUrl,
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
