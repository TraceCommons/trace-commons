import { useQuery } from "@tanstack/react-query";
import { accountScope } from "../query/scope";
import { getCoreStatus } from "./core-api";
import { coreKeys } from "./query-keys";
import type { CoreStatusState } from "./types";

export function useCoreStatus() {
  const query = useQuery({
    queryKey: coreKeys.status,
    queryFn: getCoreStatus,
  });

  return {
    ...query,
    data: query.data ?? null,
    tenantId: query.data?.daemon.tenant_id ?? null,
    scope: accountScope(query.data?.daemon.tenant_id),
    state: (query.isPending
      ? "loading"
      : query.isError
        ? "error"
        : "ready") as CoreStatusState,
    refresh: async () => {
      await query.refetch();
    },
  };
}
