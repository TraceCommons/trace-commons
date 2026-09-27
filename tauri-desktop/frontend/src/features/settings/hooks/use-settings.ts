import { useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../api/query-keys";
import { getSettings } from "../api/settings-api";

export function useSettings() {
  const core = useCoreStatus();
  const query = useQuery({
    queryKey: settingsKeys.snapshot(core.scope),
    queryFn: getSettings,
    enabled: core.isSuccess,
  });

  return {
    ...query,
    data: query.data ?? null,
    state: (query.isPending ? "loading" : query.isError ? "error" : "ready") as
      | "loading"
      | "error"
      | "ready",
    refresh: async () => {
      await query.refetch();
    },
  };
}
