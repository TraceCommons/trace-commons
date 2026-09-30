import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import {
  evaluateComparisonSpecification,
  listComparisonSpecifications,
  previewComparisonSpecification,
  saveComparisonSpecification,
} from "../api/comparison-specifications-api";
import { insightsKeys } from "../api/query-keys";
import type {
  ComparisonResult,
  ComparisonSpecification,
  ComparisonSpecificationInput,
} from "../specifications";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Specification workflow owns query, preview, save, and evaluation state.
export function useComparisonSpecifications() {
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const specificationsQuery = useQuery({
    queryKey: insightsKeys.comparisonSpecifications(),
    queryFn: listComparisonSpecifications,
  });
  const selectedQuery = useQuery({
    queryKey: insightsKeys.comparisonSpecification(selectedId ?? "none"),
    queryFn: async () => {
      const specifications = await queryClient.ensureQueryData({
        queryKey: insightsKeys.comparisonSpecifications(),
        queryFn: listComparisonSpecifications,
      });
      const selected = specifications.find((item) => item.id === selectedId);
      if (!selected) throw new Error("Specification not found");
      return selected;
    },
    enabled: selectedId !== null,
  });
  const previewMutation = useMutation({
    mutationFn: previewComparisonSpecification,
  });
  const saveMutation = useMutation({
    mutationFn: saveComparisonSpecification,
    onSuccess: async (specification) => {
      setSelectedId(specification.id);
      queryClient.setQueryData(
        insightsKeys.comparisonSpecification(specification.id),
        specification,
      );
      await queryClient.invalidateQueries({
        queryKey: insightsKeys.comparisonSpecifications(),
      });
    },
  });
  const evaluateMutation = useMutation({
    mutationFn: evaluateComparisonSpecification,
  });
  const refresh = useCallback(async () => {
    await specificationsQuery.refetch();
  }, [specificationsQuery.refetch]);
  const calculate = useCallback(
    async (input: ComparisonSpecificationInput) => {
      setActionError(null);
      try {
        await previewMutation.mutateAsync(input);
      } catch {
        setActionError(
          "Specification preview unavailable. Check task evidence and date range.",
        );
      }
    },
    [previewMutation],
  );
  const save = useCallback(
    async (input: ComparisonSpecificationInput) => {
      setActionError(null);
      try {
        await saveMutation.mutateAsync(input);
      } catch {
        setActionError("Specification was not saved.");
      }
    },
    [saveMutation],
  );
  const evaluate = useCallback(
    async (id: string) => {
      setSelectedId(id);
      setActionError(null);
      try {
        await evaluateMutation.mutateAsync(id);
      } catch {
        setActionError(
          "Specification evaluation unavailable. Refresh saved evidence.",
        );
      }
    },
    [evaluateMutation],
  );
  const select = useCallback(
    (specification: ComparisonSpecification) => {
      setSelectedId(specification.id);
      setActionError(null);
      evaluateMutation.reset();
    },
    [evaluateMutation],
  );
  const busy =
    specificationsQuery.isFetching ||
    selectedQuery.isFetching ||
    previewMutation.isPending ||
    saveMutation.isPending ||
    evaluateMutation.isPending;
  const state = busy
    ? "busy"
    : specificationsQuery.isPending
      ? "loading"
      : specificationsQuery.isError || selectedQuery.isError || actionError
        ? "error"
        : "ready";
  const error = actionError
    ? actionError
    : specificationsQuery.isError || selectedQuery.isError
      ? "Comparison specifications unavailable. Create reviewed comparison tasks first."
      : null;
  return {
    ...specificationsQuery,
    specifications: specificationsQuery.data ?? [],
    selected: selectedQuery.data ?? null,
    preview: previewMutation.data as {
      specification: ComparisonSpecification;
      result: ComparisonResult;
    } | null,
    result: evaluateMutation.data ?? null,
    state: state as "loading" | "ready" | "busy" | "error",
    error,
    refresh,
    calculate,
    save,
    evaluate,
    select,
    specificationsQuery,
    selectedQuery,
    previewMutation,
    saveMutation,
    evaluateMutation,
  };
}
