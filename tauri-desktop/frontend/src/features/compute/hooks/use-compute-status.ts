import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../../settings/public";
import { changeCompute, getComputeStatus } from "../api/compute-api";
import { computeKeys } from "../api/query-keys";

export function useComputeStatus() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: computeKeys.status(core.scope),
    queryFn: getComputeStatus,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: ({
      name,
      allowance,
    }: {
      name:
        | "enable_compute"
        | "resume_compute"
        | "pause_compute"
        | "disable_compute";
      allowance?: number;
    }) => changeCompute(name, allowance),
    onSuccess: async (snapshot) => {
      queryClient.setQueryData(computeKeys.status(core.scope), snapshot);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });

  async function command(
    name:
      | "enable_compute"
      | "resume_compute"
      | "pause_compute"
      | "disable_compute",
    allowance?: number,
  ) {
    try {
      await mutation.mutateAsync({ name, allowance });
    } catch {
      // The page keeps the last successful snapshot and renders this message.
    }
  }

  return {
    ...query,
    data: query.data ?? null,
    state: mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError || mutation.isError
          ? "error"
          : "ready",
    error: query.isError
      ? "Compute settings unavailable."
      : mutation.isError
        ? "Compute command failed. Previous state remains authoritative."
        : null,
    refresh: async () => {
      await query.refetch();
    },
    command,
    mutation,
  };
}
