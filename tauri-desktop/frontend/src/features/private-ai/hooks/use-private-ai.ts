import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../../settings/public";
import { waitingKeys } from "../../waiting/public";
import {
  type BalanceStatus,
  type CredentialStatus,
  cancelCredential,
  type FundingStatus,
  forgetCredential,
  getBalance,
  getCredentialStatus,
  getFunding,
  openExternalUrl,
  setPrivateInference,
  startCredential,
} from "../api/private-ai-api";
import { privateAiKeys } from "../api/query-keys";

export function usePrivateAi() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [actionError, setActionError] = useState<string | null>(null);
  const [verifiedFundingUrl, setVerifiedFundingUrl] = useState<string | null>(
    null,
  );
  const [browserUrl, setBrowserUrl] = useState<string | null>(null);
  const credentialQuery = useQuery<CredentialStatus>({
    queryKey: privateAiKeys.credential(core.scope),
    queryFn: getCredentialStatus,
    enabled: core.isSuccess,
    refetchInterval: (query) =>
      query.state.data?.state === "obtaining" ? 2000 : false,
  });
  const balanceQuery = useQuery<BalanceStatus>({
    queryKey: privateAiKeys.balance(core.scope),
    queryFn: getBalance,
    enabled: false,
  });
  const fundingQuery = useQuery<FundingStatus>({
    queryKey: privateAiKeys.funding(core.scope),
    queryFn: () => getFunding(),
    enabled: false,
  });
  const setEnabledMutation = useMutation({
    mutationFn: setPrivateInference,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
      ]);
    },
  });
  const startMutation = useMutation({
    mutationFn: startCredential,
    onSuccess: async (result) => {
      setBrowserUrl(result.browserUrl);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: privateAiKeys.credential(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });
  const cancelMutation = useMutation({
    mutationFn: cancelCredential,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: privateAiKeys.credential(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });
  const forgetMutation = useMutation({
    mutationFn: forgetCredential,
    onSuccess: async () => {
      setBrowserUrl(null);
      setVerifiedFundingUrl(null);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: privateAiKeys.credential(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: privateAiKeys.balance(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: privateAiKeys.funding(core.scope),
        }),
      ]);
    },
  });
  const verifyFundingMutation = useMutation({
    mutationFn: (input: {
      organizationId: string;
      connectionRevision: string;
    }) => getFunding(input),
    onSuccess: (result) => {
      queryClient.setQueryData(privateAiKeys.funding(core.scope), result);
      setVerifiedFundingUrl(result.browserUrl ?? null);
    },
  });
  const openBrowserMutation = useMutation({
    mutationFn: openExternalUrl,
  });

  useEffect(() => {
    if (credentialQuery.data?.state !== "obtaining") {
      setBrowserUrl(null);
    }
  }, [credentialQuery.data?.state]);

  const refreshCredential = useCallback(async () => {
    await credentialQuery.refetch();
  }, [credentialQuery.refetch]);
  const refreshBalance = useCallback(async () => {
    await balanceQuery.refetch();
  }, [balanceQuery.refetch]);
  const refreshFunding = useCallback(async () => {
    setVerifiedFundingUrl(null);
    await fundingQuery.refetch();
  }, [fundingQuery.refetch]);
  const verifyFunding = useCallback(async () => {
    const funding = fundingQuery.data;
    if (!funding?.organizationId || !funding.connectionRevision) return;
    setActionError(null);
    try {
      await verifyFundingMutation.mutateAsync({
        organizationId: funding.organizationId,
        connectionRevision: funding.connectionRevision,
      });
    } catch {
      setActionError(
        "Funding destination changed. Refresh account and try again.",
      );
    }
  }, [fundingQuery.data, verifyFundingMutation]);
  const setEnabled = useCallback(
    async (enabled: boolean) => {
      setActionError(null);
      try {
        await setEnabledMutation.mutateAsync(enabled);
      } catch {
        setActionError(
          "Private inference setting was not confirmed. Refresh status before retrying.",
        );
      }
    },
    [setEnabledMutation],
  );
  const start = useCallback(
    async (provider: string) => {
      setActionError(null);
      try {
        await startMutation.mutateAsync(provider);
      } catch {
        setActionError(
          "Credential ceremony did not start. Check provider availability.",
        );
      }
    },
    [startMutation],
  );
  const cancel = useCallback(async () => {
    setActionError(null);
    try {
      await cancelMutation.mutateAsync();
    } catch {
      setActionError("Credential ceremony was not cancelled.");
    }
  }, [cancelMutation]);
  const forget = useCallback(async () => {
    setActionError(null);
    try {
      await forgetMutation.mutateAsync();
    } catch {
      setActionError("Credential was not forgotten locally.");
    }
  }, [forgetMutation]);
  const openBrowser = useCallback(
    async (url: string) => {
      setActionError(null);
      try {
        await openBrowserMutation.mutateAsync(url);
      } catch {
        setActionError("Browser destination could not be opened.");
      }
    },
    [openBrowserMutation],
  );

  const isBusy =
    setEnabledMutation.isPending ||
    startMutation.isPending ||
    cancelMutation.isPending ||
    forgetMutation.isPending ||
    verifyFundingMutation.isPending ||
    openBrowserMutation.isPending ||
    balanceQuery.isFetching ||
    fundingQuery.isFetching;
  return {
    busy: isBusy,
    error: actionError,
    credential: credentialQuery.data ?? null,
    balance: balanceQuery.data ?? null,
    funding: fundingQuery.data ?? null,
    verifiedFundingUrl,
    browserUrl,
    refreshCredential,
    refreshBalance,
    refreshFunding,
    verifyFunding,
    setEnabled,
    start,
    cancel,
    forget,
    openBrowser,
    credentialQuery,
    balanceQuery,
    fundingQuery,
    setEnabledMutation,
    startMutation,
    cancelMutation,
    forgetMutation,
    verifyFundingMutation,
    openBrowserMutation,
  };
}
