import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { requestHistoryRefresh } from "../api/history-api";
import { historyKeys } from "../api/query-keys";

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
      <Button
        className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
        type="button"
        onClick={() => refresh.mutate()}
        disabled={refresh.isPending || !core.isSuccess}
      >
        {refresh.isPending ? "Requesting…" : "Request server refresh"}
      </Button>
      {refresh.isSuccess && (
        <span className="text-right text-xs text-muted-foreground" role="status">
          Refresh requested. The daemon checks server results asynchronously.
        </span>
      )}
      {refresh.isError && (
        <span className="text-right text-xs text-destructive" role="alert">
          Refresh request failed. Try again when the core is available.
        </span>
      )}
    </div>
  );
}
