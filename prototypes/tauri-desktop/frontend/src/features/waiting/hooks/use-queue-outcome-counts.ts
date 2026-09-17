import { useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { waitingKeys } from "../api/query-keys";
import { getQueueOutcomeCounts } from "../api/waiting-api";
import type { QueueOutcomeCounts } from "../types";

export function useQueueOutcomeCounts() {
  const core = useCoreStatus();
  const query = useQuery<QueueOutcomeCounts>({
    queryKey: waitingKeys.outcomeCounts(core.scope),
    queryFn: getQueueOutcomeCounts,
    enabled: core.isSuccess,
  });
  return {
    ...query,
    data: query.data ?? null,
    state: (query.isPending ? "loading" : query.isError ? "error" : "ready") as
      | "loading"
      | "ready"
      | "error",
    refresh: async () => {
      await query.refetch();
    },
  };
}
