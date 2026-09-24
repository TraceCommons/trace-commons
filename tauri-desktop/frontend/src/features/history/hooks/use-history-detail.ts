import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { getHistoryDetail } from "../api/history-api";
import { historyKeys } from "../api/query-keys";

export function useHistoryDetail() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const core = useCoreStatus();
  const query = useQuery({
    queryKey: historyKeys.detail(core.scope, selectedId ?? "none"),
    queryFn: () => getHistoryDetail(selectedId as string),
    enabled: core.isSuccess && selectedId !== null,
  });
  const open = (submissionId: string) => {
    setSelectedId(submissionId);
  };
  return {
    ...query,
    data: query.data ?? null,
    selectedId,
    state: (selectedId === null
      ? "idle"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "idle" | "loading" | "error" | "ready",
    open,
    reload: async () => {
      if (selectedId !== null) await query.refetch();
    },
  };
}
