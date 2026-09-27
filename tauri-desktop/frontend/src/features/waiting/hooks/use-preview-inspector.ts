import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { useCallback, useEffect } from "react";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  getPreviewBodyPage,
  getPreviewTurns as getPreviewTurnsForEntry,
  type PreviewBodyPage,
  searchOriginal,
} from "../api/preview-api";
import { waitingKeys } from "../api/query-keys";

type PreviewPageParam = {
  offset: number;
  digest: string | null;
};

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Inspector hook coordinates bounded transcript, turn, and search states.
export function usePreviewInspector(entryId: string | null) {
  const core = useCoreStatus();
  const bodyQuery = useInfiniteQuery({
    queryKey: waitingKeys.previewBody(core.scope, entryId ?? "none"),
    queryFn: ({ pageParam }: { pageParam: PreviewPageParam }) =>
      getPreviewBodyPage(
        entryId as string,
        pageParam.offset,
        pageParam.digest ?? undefined,
      ),
    initialPageParam: { offset: 0, digest: null as string | null },
    getNextPageParam: (lastPage: PreviewBodyPage) =>
      lastPage.next_offset === null
        ? undefined
        : { offset: lastPage.next_offset, digest: lastPage.body_digest },
    enabled: false,
  });
  const pages = bodyQuery.data?.pages ?? [];
  const firstPage = pages[0];
  const lastPage = pages.at(-1);
  const digest = firstPage?.body_digest ?? null;
  const nextOffset = lastPage?.next_offset ?? null;
  const body = pages.map((page) => page.chunk).join("");
  const totalBytes = firstPage?.total_bytes ?? 0;
  const turnsQuery = useQuery({
    queryKey: waitingKeys.previewTurns(
      core.scope,
      entryId ?? "none",
      digest ?? "none",
    ),
    queryFn: () => getPreviewTurnsForEntry(entryId as string, digest as string),
    enabled: false,
  });
  const searchMutation = useMutation({
    mutationFn: (input: { entryId: string; needle: string }) =>
      searchOriginal(input.entryId, input.needle),
  });

  // biome-ignore lint/correctness/useExhaustiveDependencies: entryId resets inspector-local input when selected preview changes.
  useEffect(() => {
    searchMutation.reset();
  }, [entryId, searchMutation.reset]);

  const open = useCallback(async () => {
    if (!entryId || !core.isSuccess) return;
    await bodyQuery.refetch();
  }, [bodyQuery.refetch, core.isSuccess, entryId]);
  const loadMore = useCallback(async () => {
    if (!entryId || nextOffset === null || !digest) return;
    await bodyQuery.fetchNextPage();
  }, [bodyQuery.fetchNextPage, digest, entryId, nextOffset]);
  const loadTurns = useCallback(async () => {
    if (!entryId || !digest || nextOffset !== null) return;
    await turnsQuery.refetch();
  }, [digest, entryId, nextOffset, turnsQuery.refetch]);
  const search = useCallback(
    async (needle: string) => {
      if (!entryId || !needle.trim()) {
        searchMutation.reset();
        return;
      }
      await searchMutation.mutateAsync({ entryId, needle: needle.trim() });
    },
    [entryId, searchMutation.mutateAsync, searchMutation.reset],
  );

  const isBusy =
    bodyQuery.isFetching || turnsQuery.isFetching || searchMutation.isPending;
  const state = isBusy
    ? bodyQuery.data
      ? "busy"
      : "loading"
    : bodyQuery.isError
      ? "error"
      : bodyQuery.data
        ? "ready"
        : "idle";
  const error = bodyQuery.isError
    ? "Redacted transcript unavailable. Nothing is sent without approval."
    : turnsQuery.isError
      ? "Turn index unavailable; raw redacted body remains available."
      : searchMutation.isError
        ? "Original-session search unavailable. Redacted body remains local."
        : null;
  const matches =
    searchMutation.variables?.entryId === entryId
      ? (searchMutation.data ?? null)
      : null;

  return {
    ...bodyQuery,
    body,
    digest,
    nextOffset,
    totalBytes,
    turns: turnsQuery.data ?? [],
    matches,
    state: state as "idle" | "loading" | "ready" | "busy" | "error",
    error,
    open,
    loadMore,
    loadTurns,
    search,
    bodyQuery,
    turnsQuery,
    searchMutation,
  };
}
