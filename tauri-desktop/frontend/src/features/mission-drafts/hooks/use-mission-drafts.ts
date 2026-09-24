import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  deleteMissionDraft,
  importMissionDraft,
  listMissionDrafts,
  showMissionDraft,
} from "../api/mission-draft-api";
import { missionDraftKeys } from "../api/query-keys";
import type { MissionDraftReview } from "../types";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Draft inbox hook coordinates list, detail, import, and delete state.
export function useMissionDrafts() {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [lastImport, setLastImport] = useState<MissionDraftReview | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const listQuery = useQuery({
    queryKey: missionDraftKeys.list,
    queryFn: listMissionDrafts,
  });
  const detailQuery = useQuery({
    queryKey: missionDraftKeys.detail(selectedId ?? "none"),
    queryFn: () => showMissionDraft(selectedId as string),
    enabled: selectedId !== null,
  });
  const importMutation = useMutation({
    mutationFn: importMissionDraft,
    onSuccess: (review) => setLastImport(review),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: missionDraftKeys.list }),
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteMissionDraft(id),
    onSuccess: (_, id) => {
      if (selectedId === id) setSelectedId(null);
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: missionDraftKeys.list }),
  });
  const refresh = useCallback(async () => {
    await listQuery.refetch();
  }, [listQuery.refetch]);
  const importFile = useCallback(
    async (file: File) => {
      setActionError(null);
      try {
        await importMutation.mutateAsync(file);
      } catch {
        setActionError("Draft import failed. Check schema, URLs, and budget.");
      }
    },
    [importMutation],
  );
  const open = useCallback((id: string) => {
    setActionError(null);
    setSelectedId(id);
  }, []);
  const remove = useCallback(async () => {
    if (!selectedId || deleteMutation.isPending) return;
    setActionError(null);
    try {
      await deleteMutation.mutateAsync(selectedId);
    } catch {
      setActionError("Draft was not deleted.");
    }
  }, [deleteMutation, selectedId]);

  const state =
    importMutation.isPending || deleteMutation.isPending
      ? "busy"
      : listQuery.isPending || (selectedId !== null && detailQuery.isPending)
        ? "loading"
        : listQuery.isError || detailQuery.isError || actionError
          ? "error"
          : "ready";
  const error = actionError
    ? actionError
    : listQuery.isError
      ? "Mission draft inbox unavailable."
      : detailQuery.isError
        ? "Draft details unavailable. Refresh inbox."
        : null;
  return {
    ...listQuery,
    drafts: listQuery.data ?? [],
    selected: detailQuery.data ?? null,
    lastImport,
    state: state as "loading" | "ready" | "busy" | "error",
    error,
    refresh,
    importFile,
    open,
    remove,
    listQuery,
    detailQuery,
    importMutation,
    deleteMutation,
  };
}
