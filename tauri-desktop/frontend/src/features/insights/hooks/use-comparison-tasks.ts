import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  clearComparisonTaskOutcome,
  createComparisonTask,
  explainComparisonTask,
  listComparisonTasks,
  reconfirmComparisonTask,
  replaceComparisonTaskEpisodes,
  setComparisonTaskContext,
  setComparisonTaskOutcome,
} from "../api/comparison-tasks-api";
import { insightsKeys } from "../api/query-keys";
import type { ComparisonContext } from "../comparisons";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Task workflow owns detail queries and revision-checked mutations.
export function useComparisonTasks() {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const tasksQuery = useQuery({
    queryKey: insightsKeys.comparisonTasks(),
    queryFn: listComparisonTasks,
  });
  const detailQuery = useQuery({
    queryKey: insightsKeys.comparisonTask(selectedId ?? "none"),
    queryFn: () => explainComparisonTask(selectedId as string),
    enabled: selectedId !== null,
  });
  const invalidateTask = useCallback(
    (id: string) =>
      Promise.all([
        queryClient.invalidateQueries({
          queryKey: insightsKeys.comparisonTasks(),
        }),
        queryClient.invalidateQueries({
          queryKey: insightsKeys.comparisonTask(id),
        }),
      ]),
    [queryClient],
  );
  const createMutation = useMutation({
    mutationFn: createComparisonTask,
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: insightsKeys.comparisonTasks(),
      }),
  });
  const replaceMutation = useMutation({
    mutationFn: ({
      id,
      revision,
      episodeIds,
    }: {
      id: string;
      revision: number;
      episodeIds: string[];
    }) => replaceComparisonTaskEpisodes(id, revision, episodeIds),
    onSuccess: (_, variables) => invalidateTask(variables.id),
  });
  const contextMutation = useMutation({
    mutationFn: ({
      id,
      revision,
      context,
    }: {
      id: string;
      revision: number;
      context: ComparisonContext;
    }) => setComparisonTaskContext(id, revision, context),
    onSuccess: (_, variables) => invalidateTask(variables.id),
  });
  const outcomeMutation = useMutation({
    mutationFn: ({
      id,
      revision,
      value,
    }: {
      id: string;
      revision: number;
      value: string;
    }) => setComparisonTaskOutcome(id, revision, value),
    onSuccess: (_, variables) => invalidateTask(variables.id),
  });
  const clearOutcomeMutation = useMutation({
    mutationFn: ({ id, revision }: { id: string; revision: number }) =>
      clearComparisonTaskOutcome(id, revision),
    onSuccess: (_, variables) => invalidateTask(variables.id),
  });
  const reconfirmMutation = useMutation({
    mutationFn: ({
      id,
      revision,
      digest,
    }: {
      id: string;
      revision: number;
      digest: string;
    }) => reconfirmComparisonTask(id, revision, digest),
    onSuccess: (_, variables) => invalidateTask(variables.id),
  });

  const refresh = useCallback(async () => {
    await tasksQuery.refetch();
  }, [tasksQuery.refetch]);
  const open = useCallback(
    async (id: string) => {
      setActionError(null);
      setSelectedId(id);
      try {
        await queryClient.fetchQuery({
          queryKey: insightsKeys.comparisonTask(id),
          queryFn: () => explainComparisonTask(id),
        });
      } catch {
        setActionError("Task detail unavailable. Refresh tasks.");
      }
    },
    [queryClient],
  );
  const create = useCallback(
    async (ids: string[]) => {
      setActionError(null);
      try {
        await createMutation.mutateAsync(ids);
        return true;
      } catch {
        setActionError("Task was not created. Select episodes.");
        return false;
      }
    },
    [createMutation],
  );
  const replaceEpisodes = useCallback(
    async (ids: string[]): Promise<boolean> => {
      const task = detailQuery.data?.task;
      if (!task) return false;
      setActionError(null);
      try {
        await replaceMutation.mutateAsync({
          id: task.id,
          revision: task.revision,
          episodeIds: ids,
        });
        return true;
      } catch {
        setActionError(
          "Task evidence was not changed. Refresh before retrying.",
        );
        return false;
      }
    },
    [detailQuery.data?.task, replaceMutation],
  );
  const setContext = useCallback(
    async (context: ComparisonContext): Promise<boolean> => {
      const task = detailQuery.data?.task;
      if (!task) return false;
      setActionError(null);
      try {
        await contextMutation.mutateAsync({
          id: task.id,
          revision: task.revision,
          context,
        });
        return true;
      } catch {
        setActionError("Task context was not saved. Check required fields.");
        return false;
      }
    },
    [contextMutation, detailQuery.data?.task],
  );
  const setOutcome = useCallback(
    async (value: string): Promise<boolean> => {
      const task = detailQuery.data?.task;
      if (!task) return false;
      setActionError(null);
      try {
        await outcomeMutation.mutateAsync({
          id: task.id,
          revision: task.revision,
          value,
        });
        return true;
      } catch {
        setActionError("Task outcome was not saved. Refresh before retrying.");
        return false;
      }
    },
    [detailQuery.data?.task, outcomeMutation],
  );
  const clearOutcome = useCallback(async (): Promise<boolean> => {
    const task = detailQuery.data?.task;
    if (!task) return false;
    setActionError(null);
    try {
      await clearOutcomeMutation.mutateAsync({
        id: task.id,
        revision: task.revision,
      });
      return true;
    } catch {
      setActionError("Task outcome was not cleared.");
      return false;
    }
  }, [clearOutcomeMutation, detailQuery.data?.task]);
  const reconfirm = useCallback(async (): Promise<boolean> => {
    const task = detailQuery.data?.task;
    if (!task) return false;
    setActionError(null);
    try {
      await reconfirmMutation.mutateAsync({
        id: task.id,
        revision: task.revision,
        digest: task.material_digest,
      });
      return true;
    } catch {
      setActionError("Task could not be reconfirmed. Review changed evidence.");
      return false;
    }
  }, [detailQuery.data?.task, reconfirmMutation]);

  const busy =
    detailQuery.isFetching ||
    createMutation.isPending ||
    replaceMutation.isPending ||
    contextMutation.isPending ||
    outcomeMutation.isPending ||
    clearOutcomeMutation.isPending ||
    reconfirmMutation.isPending;
  const state = busy
    ? "busy"
    : tasksQuery.isPending
      ? "loading"
      : tasksQuery.isError || detailQuery.isError || actionError
        ? "error"
        : "ready";
  const error = actionError
    ? actionError
    : tasksQuery.isError
      ? "Comparison tasks unavailable. Refresh local Insights."
      : detailQuery.isError
        ? "Task detail unavailable. Refresh tasks."
        : null;
  return {
    ...tasksQuery,
    tasks: tasksQuery.data ?? [],
    detail: detailQuery.data ?? null,
    state: state as "loading" | "ready" | "busy" | "error",
    error,
    refresh,
    open,
    close: () => setSelectedId(null),
    create,
    replaceEpisodes,
    setContext,
    setOutcome,
    clearOutcome,
    reconfirm,
    tasksQuery,
    detailQuery,
    createMutation,
    replaceMutation,
    contextMutation,
    outcomeMutation,
    clearOutcomeMutation,
    reconfirmMutation,
  };
}
