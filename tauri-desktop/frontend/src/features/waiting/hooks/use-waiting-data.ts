import { useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { waitingKeys } from "../api/query-keys";
import { getWaitingData } from "../api/waiting-api";

export function useWaitingData() {
  const core = useCoreStatus();
  const query = useQuery({
    queryKey: waitingKeys.list(core.scope),
    queryFn: getWaitingData,
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
