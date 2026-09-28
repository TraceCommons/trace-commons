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
import { settingsKeys } from "../../settings/public";
import {
  disconnectInferenceConnection,
  getCurrentInferenceConnection,
  getInferenceOffers,
  installInferenceConnection,
  selectInferenceConnection,
} from "../api/onboarding-api";
import { onboardingKeys } from "../api/query-keys";
import {
  type InferenceOffer,
  type InstallTarget,
  inferenceView,
  needsAccountSignIn,
  type SelectResult,
} from "../inference-connection";

// The connect-inference step (K12): account sign-in when needed, the offers,
// select, and the separate install. Every answer is the daemon's; nothing
// here decides that a connection exists or is installed.
export function useOnboardingInference() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [justSelected, setJustSelected] = useState<SelectResult | null>(null);
  const [signInUrl, setSignInUrl] = useState<string | null>(null);
  const session = useQuery({
    queryKey: coreKeys.accountSession,
    queryFn: getAccountSessionStatus,
    enabled: isTauriRuntime(),
  });
  const signedIn = session.data?.signed_in === true;
  const offers = useQuery({
    queryKey: onboardingKeys.inferenceOffers(core.scope),
    queryFn: getInferenceOffers,
    enabled: signedIn,
    retry: false,
  });
  const current = useQuery({
    queryKey: onboardingKeys.inferenceCurrent(core.scope),
    queryFn: getCurrentInferenceConnection,
    enabled: signedIn,
    retry: false,
  });
  // A session the daemon refuses (expired, or a device bearer) is the same
  // as none: the contributor is sent to sign in, not shown a failure.
  const sessionRefused =
    needsAccountSignIn(offers.error) || needsAccountSignIn(current.error);
  const refreshConnection = () =>
    Promise.all([
      queryClient.invalidateQueries({
        queryKey: onboardingKeys.inferenceOffers(core.scope),
      }),
      queryClient.invalidateQueries({
        queryKey: onboardingKeys.inferenceCurrent(core.scope),
      }),
      // Installing or removing a witness changes what the witness
      // disclosure screen reads next.
      queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      queryClient.invalidateQueries({
        queryKey: settingsKeys.snapshot(core.scope),
      }),
    ]);
  const signIn = useMutation({
    mutationFn: async () => {
      setSignInUrl(null);
      const unlisten = await listenTauri("account-sign-in-url", (payload) => {
        setSignInUrl(typeof payload === "string" ? payload : null);
      });
      try {
        return await signInToAccount();
      } finally {
        unlisten();
      }
    },
    onSettled: async () => {
      setSignInUrl(null);
      await queryClient.invalidateQueries({ queryKey: coreKeys.accountSession });
      await refreshConnection();
    },
  });
  const select = useMutation({
    mutationFn: (input: {
      offer: InferenceOffer;
      expectedVersion: number | null;
    }) => selectInferenceConnection(input.offer, input.expectedVersion),
    onSuccess: (result) => setJustSelected(result),
    onSettled: refreshConnection,
  });
  const install = useMutation({
    mutationFn: (target: InstallTarget) => installInferenceConnection(target),
    onSettled: async () => {
      setJustSelected(null);
      await refreshConnection();
    },
  });
  const disconnect = useMutation({
    mutationFn: (connectionId: string) =>
      disconnectInferenceConnection(connectionId),
    onSettled: async () => {
      setJustSelected(null);
      await refreshConnection();
    },
  });
  const view = inferenceView({
    signedIn: sessionRefused
      ? false
      : session.data
        ? session.data.signed_in
        : session.isError
          ? false
          : null,
    offers: offers.data ?? null,
    current: current.data ?? null,
    justSelected,
    loading: offers.isFetching || current.isFetching,
  });
  const failed =
    !sessionRefused && (offers.isError || current.isError) && !offers.isFetching;
  const busy =
    signIn.isPending ||
    select.isPending ||
    install.isPending ||
    disconnect.isPending;
  return {
    view,
    failed,
    busy,
    signInUrl,
    openSignInUrl: () => {
      if (signInUrl) void openAccountSignInUrl(signInUrl);
    },
    signIn: () => signIn.mutate(),
    signInFailed: signIn.isError,
    retry: () => void refreshConnection(),
    select: (offer: InferenceOffer, expectedVersion: number | null) =>
      select.mutate({ offer, expectedVersion }),
    selectFailed: select.isError,
    previousRemoved: justSelected?.previous_witness_removed === true,
    install: (target: InstallTarget) => install.mutate(target),
    installFailed: install.isError,
    installed: install.isSuccess,
    disconnect: (connectionId: string) => disconnect.mutate(connectionId),
    disconnectPending: disconnect.data === "pending",
    disconnectFailed: disconnect.isError,
  };
}
