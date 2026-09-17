import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  analyzeInsight,
  annotateInsight,
  clearInsightAnnotation,
  deleteInsight,
  explainInsight,
  getInsightsData,
  linkGit,
  linkTestReport,
  unlinkEvidence,
} from "../api/insights-api";
import { insightsKeys } from "../api/query-keys";
import type { Insight } from "../types";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Insights hook coordinates file analysis and several evidence mutations.
export function useInsightsData() {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [transientSelected, setTransientSelected] = useState<Insight | null>(
    null,
  );
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [selectedSource, setSelectedSource] = useState("codex");
  const [selectedIsSaved, setSelectedIsSaved] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const dataQuery = useQuery({
    queryKey: insightsKeys.data(),
    queryFn: getInsightsData,
  });
  const detailQuery = useQuery({
    queryKey: insightsKeys.detail(selectedId ?? "none"),
    queryFn: () => explainInsight(selectedId as string),
    enabled: selectedIsSaved && selectedId !== null,
  });
  const cacheSavedInsight = useCallback(
    (insight: Insight) => {
      queryClient.setQueryData(insightsKeys.detail(insight.id), insight);
    },
    [queryClient],
  );
  const analyzeMutation = useMutation({
    mutationFn: ({
      file,
      source,
      save,
    }: {
      file: File;
      source: string;
      save: boolean;
    }) => analyzeInsight(file, source, save),
    onSuccess: async (insight, variables) => {
      if (variables.save) {
        setSelectedId(insight.id);
        setSelectedIsSaved(true);
        setTransientSelected(null);
        cacheSavedInsight(insight);
        await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
      } else {
        setTransientSelected(insight);
        setSelectedIsSaved(false);
      }
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteInsight(id),
    onSuccess: async (_, id) => {
      if (selectedId === id) {
        setSelectedId(null);
        setTransientSelected(null);
        setSelectedIsSaved(false);
        setSelectedFile(null);
      }
      queryClient.removeQueries({ queryKey: insightsKeys.detail(id) });
      await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
    },
  });
  const annotateMutation = useMutation({
    mutationFn: ({
      id,
      category,
      outcome,
    }: {
      id: string;
      category: string;
      outcome: string;
    }) => annotateInsight(id, category, outcome),
    onSuccess: async (insight) => {
      cacheSavedInsight(insight);
      await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
    },
  });
  const clearAnnotationMutation = useMutation({
    mutationFn: (id: string) => clearInsightAnnotation(id),
    onSuccess: async (insight) => {
      cacheSavedInsight(insight);
      await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
    },
  });
  const linkReportMutation = useMutation({
    mutationFn: ({ id, file }: { id: string; file: File }) =>
      linkTestReport(id, file),
    onSuccess: async (insight) => {
      cacheSavedInsight(insight);
      await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
    },
  });
  const linkGitMutation = useMutation({
    mutationFn: ({
      id,
      repository,
      commit,
    }: {
      id: string;
      repository: string;
      commit: string;
    }) => linkGit(id, repository, commit),
    onSuccess: async (insight) => {
      cacheSavedInsight(insight);
      await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
    },
  });
  const unlinkMutation = useMutation({
    mutationFn: ({ id, evidenceId }: { id: string; evidenceId: string }) =>
      unlinkEvidence(id, evidenceId),
    onSuccess: async (insight) => {
      cacheSavedInsight(insight);
      await queryClient.invalidateQueries({ queryKey: insightsKeys.data() });
    },
  });

  const refresh = useCallback(async () => {
    await dataQuery.refetch();
  }, [dataQuery.refetch]);
  const analyze = useCallback(
    async (file: File, source: string, save = false) => {
      setActionError(null);
      setSelectedFile(file);
      setSelectedSource(source);
      try {
        await analyzeMutation.mutateAsync({ file, source, save });
      } catch {
        setActionError(
          "Analysis failed. Check source format and file contents.",
        );
      }
    },
    [analyzeMutation],
  );
  const openSnapshot = useCallback(
    async (id: string) => {
      setActionError(null);
      setSelectedId(id);
      setSelectedIsSaved(true);
      setTransientSelected(null);
      setSelectedFile(null);
      try {
        await queryClient.fetchQuery({
          queryKey: insightsKeys.detail(id),
          queryFn: () => explainInsight(id),
        });
      } catch {
        setActionError("Saved snapshot unavailable. Refresh history.");
      }
    },
    [queryClient],
  );
  const save = useCallback(async () => {
    if (!selectedFile) return;
    await analyze(selectedFile, selectedSource, true);
  }, [analyze, selectedFile, selectedSource]);
  const remove = useCallback(async () => {
    if (!selectedId || !selectedIsSaved || deleteMutation.isPending) return;
    setActionError(null);
    try {
      await deleteMutation.mutateAsync(selectedId);
    } catch {
      setActionError("Saved snapshot was not deleted.");
    }
  }, [deleteMutation, selectedId, selectedIsSaved]);
  const annotate = useCallback(
    async (category: string, outcome: string): Promise<boolean> => {
      if (!selectedId || !selectedIsSaved) return false;
      setActionError(null);
      try {
        await annotateMutation.mutateAsync({
          id: selectedId,
          category,
          outcome,
        });
        return true;
      } catch {
        setActionError("Assessment was not saved.");
        return false;
      }
    },
    [annotateMutation, selectedId, selectedIsSaved],
  );
  const clearAnnotation = useCallback(async (): Promise<boolean> => {
    if (!selectedId || !selectedIsSaved) return false;
    setActionError(null);
    try {
      await clearAnnotationMutation.mutateAsync(selectedId);
      return true;
    } catch {
      setActionError("Assessment was not cleared.");
      return false;
    }
  }, [clearAnnotationMutation, selectedId, selectedIsSaved]);
  const linkReport = useCallback(
    async (file: File) => {
      if (!selectedId || !selectedIsSaved) return;
      setActionError(null);
      try {
        await linkReportMutation.mutateAsync({ id: selectedId, file });
      } catch {
        setActionError(
          "Test report was not linked. Use a supported report under 64 KiB.",
        );
      }
    },
    [linkReportMutation, selectedId, selectedIsSaved],
  );
  const linkRepository = useCallback(
    async (repository: string, commit: string): Promise<boolean> => {
      if (!selectedId || !selectedIsSaved) return false;
      setActionError(null);
      try {
        await linkGitMutation.mutateAsync({
          id: selectedId,
          repository,
          commit,
        });
        return true;
      } catch {
        setActionError(
          "Git commit was not linked. Check repository and commit.",
        );
        return false;
      }
    },
    [linkGitMutation, selectedId, selectedIsSaved],
  );
  const unlink = useCallback(
    async (evidenceId: string) => {
      if (!selectedId || !selectedIsSaved) return;
      setActionError(null);
      try {
        await unlinkMutation.mutateAsync({ id: selectedId, evidenceId });
      } catch {
        setActionError("Evidence link was not removed. Refresh saved history.");
      }
    },
    [selectedId, selectedIsSaved, unlinkMutation],
  );

  const mutationBusy = [
    analyzeMutation,
    deleteMutation,
    annotateMutation,
    clearAnnotationMutation,
    linkReportMutation,
    linkGitMutation,
    unlinkMutation,
  ].some((mutation) => mutation.isPending);
  const state =
    mutationBusy || detailQuery.isFetching
      ? "busy"
      : dataQuery.isPending
        ? "loading"
        : dataQuery.isError || detailQuery.isError || actionError
          ? "error"
          : "ready";
  const error = actionError
    ? actionError
    : dataQuery.isError
      ? "Local Insights unavailable. Start Tauri and refresh saved history."
      : detailQuery.isError
        ? "Saved snapshot unavailable. Refresh history."
        : null;
  return {
    ...dataQuery,
    data: dataQuery.data ?? null,
    selected: selectedIsSaved ? (detailQuery.data ?? null) : transientSelected,
    selectedIsSaved,
    state: state as "loading" | "ready" | "error" | "busy",
    error,
    refresh,
    analyze,
    openSnapshot,
    save,
    remove,
    annotate,
    clearAnnotation,
    linkReport,
    linkRepository,
    unlink,
    dataQuery,
    detailQuery,
    analyzeMutation,
    deleteMutation,
    annotateMutation,
    clearAnnotationMutation,
    linkReportMutation,
    linkGitMutation,
    unlinkMutation,
  };
}
