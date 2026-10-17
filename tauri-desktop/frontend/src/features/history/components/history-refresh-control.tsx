import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { requestHistoryRefresh } from "../api/history-api";
import { historyKeys } from "../api/query-keys";
import { TertiaryLink } from "@/design-system";

export function HistoryRefreshControl() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const refresh = useMutation({
    mutationFn: requestHistoryRefresh,
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: historyKeys.list(core.scope),
      });
    },
  });

  return (
    <div className="grid justify-items-end gap-1">
      <TertiaryLink
        type="button"
        onClick={() => refresh.mutate()}
        disabled={refresh.isPending || !core.isSuccess}
      >
        {refresh.isPending ? "Requesting…" : "Request server refresh"}
      </TertiaryLink>
      {refresh.isSuccess && (
        <span className="text-right text-xs text-tc-secondary" role="status">
          Refresh requested. The daemon checks server results asynchronously.
        </span>
      )}
      {refresh.isError && (
        <span className="text-right text-xs text-tc-outside" role="alert">
          Refresh request failed. Try again when the core is available.
        </span>
      )}
    </div>
  );
}
