import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  commitHarness,
  getHarnessList,
  type HarnessPlan,
  planHarness,
} from "../api/harness-api";
import { privateAiKeys } from "../api/query-keys";

export function useHarnesses() {
  const queryClient = useQueryClient();
  const [plan, setPlan] = useState<HarnessPlan | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const query = useQuery({
    queryKey: privateAiKeys.harnesses(),
    queryFn: getHarnessList,
  });
  const planMutation = useMutation({
    mutationFn: ({
      id,
      action,
    }: {
      id: string;
      action: "connect" | "disconnect";
    }) => planHarness(id, action),
    onSuccess: (nextPlan) => setPlan(nextPlan),
  });
  const commitMutation = useMutation({
    mutationFn: (planId: string) => commitHarness(planId),
    onSuccess: async () => {
      setPlan(null);
      await queryClient.invalidateQueries({
        queryKey: privateAiKeys.harnesses(),
      });
    },
  });
  const refresh = useCallback(async () => {
    await query.refetch();
  }, [query.refetch]);
  const prepare = useCallback(
    async (id: string, action: "connect" | "disconnect") => {
      setActionError(null);
      try {
        await planMutation.mutateAsync({ id, action });
      } catch {
        setActionError("Could not prepare tool change.");
      }
    },
    [planMutation],
  );
  const commit = useCallback(async () => {
    if (!plan?.plan_id || !plan.view.can_commit) return;
    setActionError(null);
    try {
      await commitMutation.mutateAsync(plan.plan_id);
    } catch {
      setActionError("Tool change was not committed. Prepare a new preview.");
    }
  }, [commitMutation, plan]);
  const actionBusy = planMutation.isPending || commitMutation.isPending;
  return {
    ...query,
    data: query.data ?? null,
    state: (query.isPending ? "loading" : query.isError ? "error" : "ready") as
      | "loading"
      | "ready"
      | "error",
    refresh,
    plan,
    prepare,
    commit,
    cancel: () => setPlan(null),
    actionState: (actionBusy
      ? "busy"
      : planMutation.isError || commitMutation.isError || actionError
        ? "error"
        : "idle") as "idle" | "busy" | "error",
    error: actionError,
    query,
    planMutation,
    commitMutation,
  };
}
