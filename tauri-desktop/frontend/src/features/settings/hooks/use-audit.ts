import { useQuery } from "@tanstack/react-query";
import { useShellStatusLines } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { getAudit } from "../api/audit-api";
import { settingsKeys } from "../api/query-keys";

export function useAudit() {
  const lines = useShellStatusLines();
  const core = useCoreStatus();
  const query = useQuery({
    queryKey: settingsKeys.audit(core.scope),
    queryFn: getAudit,
    enabled: core.isSuccess,
  });
  return {
    ...query,
    entries: query.data ?? [],
    state: (query.isPending ? "loading" : query.isError ? "error" : "ready") as
      | "loading"
      | "error"
      | "ready",
    error: query.isError
      ? lines.readUnavailable
      : null,
    refresh: async () => {
      await query.refetch();
    },
  };
}
