import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  annotateEpisode,
  clearEpisodeAssessment,
  createEpisode,
  deleteEpisode,
  explainEpisode,
  getQuestionCards,
  listEpisodes,
} from "../api/insights-workflows-api";
import { insightsKeys } from "../api/query-keys";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Workflow hook owns episode, detail, card, and revision-checked mutation state.
export function useInsightsWorkflows() {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const episodesQuery = useQuery({
    queryKey: insightsKeys.episodes(),
    queryFn: listEpisodes,
  });
  const detailQuery = useQuery({
    queryKey: insightsKeys.episode(selectedId ?? "none"),
    queryFn: () => explainEpisode(selectedId as string),
    enabled: selectedId !== null,
  });
  const invalidateEpisode = useCallback(
    (id: string) =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: insightsKeys.episodes() }),
        queryClient.invalidateQueries({ queryKey: insightsKeys.episode(id) }),
      ]),
    [queryClient],
  );
  const createMutation = useMutation({
    mutationFn: createEpisode,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: insightsKeys.episodes() }),
  });
  const annotateMutation = useMutation({
    mutationFn: ({
      id,
      revision,
      category,
      outcome,
    }: {
      id: string;
      revision: number;
      category: string;
      outcome: string;
    }) => annotateEpisode(id, revision, category, outcome),
    onSuccess: (_, variables) => invalidateEpisode(variables.id),
  });
  const clearMutation = useMutation({
    mutationFn: ({ id, revision }: { id: string; revision: number }) =>
      clearEpisodeAssessment(id, revision),
    onSuccess: (_, variables) => invalidateEpisode(variables.id),
  });
  const deleteMutation = useMutation({
    mutationFn: ({ id, revision }: { id: string; revision: number }) =>
      deleteEpisode(id, revision),
    onSuccess: async (_, variables) => {
      if (selectedId === variables.id) setSelectedId(null);
      queryClient.removeQueries({
        queryKey: insightsKeys.episode(variables.id),
      });
      await queryClient.invalidateQueries({
        queryKey: insightsKeys.episodes(),
      });
    },
  });
  const cardsMutation = useMutation({
    mutationFn: ({
      snapshotIds,
      episodeIds,
    }: {
      snapshotIds: string[];
      episodeIds: string[];
    }) => getQuestionCards(snapshotIds, episodeIds),
  });

  const refresh = useCallback(async () => {
    await episodesQuery.refetch();
  }, [episodesQuery.refetch]);
  const open = useCallback(
    async (id: string) => {
      setActionError(null);
      setSelectedId(id);
      try {
        await queryClient.fetchQuery({
          queryKey: insightsKeys.episode(id),
          queryFn: () => explainEpisode(id),
        });
      } catch {
        setActionError("Episode detail unavailable. Refresh episodes.");
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
        setActionError("Episode was not created. Select saved snapshots.");
        return false;
      }
    },
    [createMutation],
  );
  const annotate = useCallback(
    async (category: string, outcome: string): Promise<boolean> => {
      const episode = detailQuery.data?.episode;
      if (!episode) return false;
      setActionError(null);
      try {
        await annotateMutation.mutateAsync({
          id: episode.id,
          revision: episode.revision,
          category,
          outcome,
        });
        return true;
      } catch {
        setActionError(
          "Episode assessment was not saved. Refresh before retrying.",
        );
        return false;
      }
    },
    [annotateMutation, detailQuery.data?.episode],
  );
  const clearAssessment = useCallback(async (): Promise<boolean> => {
    const episode = detailQuery.data?.episode;
    if (!episode) return false;
    setActionError(null);
    try {
      await clearMutation.mutateAsync({
        id: episode.id,
        revision: episode.revision,
      });
      return true;
    } catch {
      setActionError("Episode assessment was not cleared.");
      return false;
    }
  }, [clearMutation, detailQuery.data?.episode]);
  const remove = useCallback(async () => {
    const episode = detailQuery.data?.episode;
    if (!episode) return;
    setActionError(null);
    try {
      await deleteMutation.mutateAsync({
        id: episode.id,
        revision: episode.revision,
      });
    } catch {
      setActionError("Episode was not deleted. Refresh before retrying.");
    }
  }, [deleteMutation, detailQuery.data?.episode]);
  const calculateCards = useCallback(
    async (snapshotIds: string[], episodeIds: string[]) => {
      setActionError(null);
      try {
        await cardsMutation.mutateAsync({ snapshotIds, episodeIds });
      } catch {
        setActionError("Question cards unavailable for selected evidence.");
      }
    },
    [cardsMutation],
  );
  const busy =
    detailQuery.isFetching ||
    createMutation.isPending ||
    annotateMutation.isPending ||
    clearMutation.isPending ||
    deleteMutation.isPending ||
    cardsMutation.isPending;
  const state = busy
    ? "busy"
    : episodesQuery.isPending
      ? "loading"
      : episodesQuery.isError || detailQuery.isError || actionError
        ? "error"
        : "ready";
  const error = actionError
    ? actionError
    : episodesQuery.isError
      ? "Episodes unavailable. Start Tauri and refresh local history."
      : detailQuery.isError
        ? "Episode detail unavailable. Refresh episodes."
        : null;
  return {
    ...episodesQuery,
    episodes: episodesQuery.data ?? [],
    detail: detailQuery.data ?? null,
    cards: cardsMutation.data ?? null,
    state: state as "loading" | "ready" | "busy" | "error",
    error,
    refresh,
    open,
    close: () => setSelectedId(null),
    create,
    annotate,
    clearAssessment,
    remove,
    calculateCards,
    episodesQuery,
    detailQuery,
    createMutation,
    annotateMutation,
    clearMutation,
    deleteMutation,
    cardsMutation,
  };
}
