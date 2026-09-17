import { useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { getHistoryData } from "../api/history-api";
import { historyKeys } from "../api/query-keys";

export function useHistoryData() {
  const core = useCoreStatus();
  const query = useQuery({
    queryKey: historyKeys.list(core.scope),
    queryFn: getHistoryData,
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
