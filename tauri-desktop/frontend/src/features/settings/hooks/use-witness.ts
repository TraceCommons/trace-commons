import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../api/query-keys";
import {
  clearWitness,
  configureWitness,
  getWitnessStatus,
  type WitnessStatus,
} from "../api/witness-api";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Witness hook maps safety-specific status and configure/clear actions.
export function useWitness() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const query = useQuery<WitnessStatus>({
    queryKey: settingsKeys.witness(core.scope),
    queryFn: getWitnessStatus,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: (
      input:
        | {
            action: "configure";
            url: string;
            signingAddress: string;
            measurements: string[];
          }
        | { action: "clear" },
    ) =>
      input.action === "configure"
        ? configureWitness(input.url, input.signingAddress, input.measurements)
        : clearWitness(),
    onSuccess: async (data) => {
      queryClient.setQueryData(settingsKeys.witness(core.scope), data);
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      ]);
    },
  });

  return {
    ...query,
    data: query.data ?? null,
    state: (mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "busy" | "loading" | "ready" | "error",
    error: query.isError
      ? "Witness status unavailable. Refresh after Rust core starts."
      : mutation.isError &&
          mutation.error instanceof Error &&
          mutation.error.message.includes("pin")
        ? "Witness configuration needs at least one valid measurement pin."
        : mutation.isError
          ? mutation.variables?.action === "clear"
            ? "Witness configuration was not cleared."
            : "Witness configuration was not saved."
          : null,
    refresh: async () => {
      await query.refetch();
    },
    configure: (url: string, signingAddress: string, measurements: string[]) =>
      mutation.mutateAsync({
        action: "configure",
        url,
        signingAddress,
        measurements,
      }),
    clear: () => mutation.mutateAsync({ action: "clear" }),
    mutation,
  };
}
