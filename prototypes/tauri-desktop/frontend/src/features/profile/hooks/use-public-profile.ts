import { useQuery } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { getPublicProfile } from "../api/profile-api";
import { profileKeys } from "../api/query-keys";

export function usePublicProfile() {
  const core = useCoreStatus();
  const query = useQuery({
    queryKey: profileKeys.public(core.scope),
    queryFn: getPublicProfile,
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
