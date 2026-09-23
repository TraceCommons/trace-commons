import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  cleanTokenStorage,
  getTokenStorage,
  setInferenceEvidence,
  setTokenCapture,
  setTokenContribution,
  type TokenStorage,
} from "../api/privacy-api";
import { settingsKeys } from "../api/query-keys";

type PrivacyAction =
  | { kind: "inference"; enabled: boolean; confirmed: boolean }
  | { kind: "contribution"; enabled: boolean; confirmed: boolean }
  | { kind: "capture"; enabled: boolean; confirmed: boolean }
  | { kind: "cleanup"; discard: boolean; confirmed: boolean };

export function usePrivacyControls() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const query = useQuery<TokenStorage>({
    queryKey: settingsKeys.tokenStorage(core.scope),
    queryFn: getTokenStorage,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: (action: PrivacyAction) => {
      switch (action.kind) {
        case "inference":
          return setInferenceEvidence(action.enabled, action.confirmed);
        case "contribution":
          return setTokenContribution(action.enabled, action.confirmed);
        case "capture":
          return setTokenCapture(action.enabled, action.confirmed);
        case "cleanup":
          return cleanTokenStorage(action.discard, action.confirmed);
      }
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: settingsKeys.tokenStorage(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      ]);
    },
  });

  async function run(action: PrivacyAction) {
    try {
      await mutation.mutateAsync(action);
    } catch {
      // Mutation state supplies the panel error.
    }
  }

  const mutationError = mutation.error;
  return {
    ...query,
    storage: query.data ?? null,
    state: (mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "busy" | "loading" | "ready" | "error",
    error: query.isError
      ? "Privacy storage unavailable. Refresh after Rust core starts."
      : mutationError instanceof Error &&
          mutationError.message.includes("disclosure")
        ? "Read the disclosure before enabling this option."
        : mutation.isError
          ? "Privacy setting was not changed."
          : null,
    refreshStorage: async () => {
      await query.refetch();
    },
    setInferenceEvidence: (enabled: boolean, confirmed: boolean) =>
      run({ kind: "inference", enabled, confirmed }),
    setTokenContribution: (enabled: boolean, confirmed: boolean) =>
      run({ kind: "contribution", enabled, confirmed }),
    setTokenCapture: (enabled: boolean, confirmed: boolean) =>
      run({ kind: "capture", enabled, confirmed }),
    cleanTokenStorage: (discard: boolean, confirmed: boolean) =>
      run({ kind: "cleanup", discard, confirmed }),
    mutation,
  };
}
