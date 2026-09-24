import { useQueryClient } from "@tanstack/react-query";
import { useCoreStatus } from "../tauri/use-core-status";
import { accountScope } from "./scope";

export function useAccountScope() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const tenantId = core.data?.daemon.tenant_id ?? null;

  return {
    ...core,
    tenantId,
    scope: accountScope(tenantId),
    clearAccountCache: () => {
      queryClient.removeQueries({
        predicate: (query) => query.queryKey[0] === "account",
      });
    },
  };
}
