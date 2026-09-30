import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../api/query-keys";
import {
  configureRouting,
  discoverRouting,
  probeRoutedTools,
  probeRouting,
  type RoutingDiscovery,
} from "../api/routing-api";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Routing hook coordinates discovery and sequential probe/configure mutations.
export function useRouting() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const discoveryQuery = useQuery<RoutingDiscovery>({
    queryKey: settingsKeys.routingDiscovery(),
    queryFn: discoverRouting,
  });
  const checkMutation = useMutation({
    mutationFn: async ({
      port,
      tokenDir,
    }: {
      port: number;
      tokenDir: string;
    }) => {
      await probeRouting(port, tokenDir);
      return probeRoutedTools(port, tokenDir);
    },
  });
  const configureMutation = useMutation({
    mutationFn: ({
      enabled,
      port,
      tokenDir,
    }: {
      enabled: boolean;
      port: number;
      tokenDir: string;
    }) => configureRouting(enabled, port, tokenDir),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: settingsKeys.routingDiscovery(),
        }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      ]);
    },
  });

  return {
    ...discoveryQuery,
    discovery: discoveryQuery.data ?? null,
    evidence: checkMutation.data ?? null,
    state: (checkMutation.isPending || configureMutation.isPending
      ? "busy"
      : discoveryQuery.isPending
        ? "loading"
        : discoveryQuery.isError ||
            checkMutation.isError ||
            configureMutation.isError
          ? "error"
          : "ready") as "busy" | "loading" | "ready" | "error",
    error: discoveryQuery.isError
      ? "Routing discovery unavailable. Refresh after Rust core starts."
      : checkMutation.isError
        ? "Routing check unavailable. Verify port and token directory."
        : configureMutation.isError
          ? "Routing declaration was not changed."
          : null,
    refresh: async () => {
      await discoveryQuery.refetch();
    },
    check: (port: number, tokenDir: string) =>
      checkMutation.mutateAsync({ port, tokenDir }),
    configure: (enabled: boolean, port: number, tokenDir: string) =>
      configureMutation.mutateAsync({ enabled, port, tokenDir }),
    checkMutation,
    configureMutation,
  };
}
