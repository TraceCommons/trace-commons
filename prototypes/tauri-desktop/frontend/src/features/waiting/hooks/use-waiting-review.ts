import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { waitingKeys } from "../api/query-keys";
import type { UndoScope } from "../api/undo-api";
import {
  approveWaitingEntry,
  dismissWaitingEntry,
  previewWaitingEntry,
} from "../api/waiting-api";
import type { WaitingPreview } from "../types";

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Review hook coordinates preview, approval, dismissal, and undo handoff states.
export function useWaitingReview(onApproved?: (scope: UndoScope) => void) {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const previewQuery = useQuery<WaitingPreview>({
    queryKey: waitingKeys.preview(core.scope, selectedId ?? "none"),
    queryFn: () => previewWaitingEntry(selectedId as string),
    enabled: core.isSuccess && selectedId !== null,
  });
  const dismissMutation = useMutation({
    mutationFn: (entryId: string) => dismissWaitingEntry(entryId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: waitingKeys.list(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.arming(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
      ]);
    },
  });
  const approveMutation = useMutation({
    mutationFn: (entryId: string) => approveWaitingEntry(entryId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: waitingKeys.list(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.arming(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
      ]);
    },
  });

  const review = (entryId: string) => {
    setSelectedId(entryId);
    setActionError(null);
  };

  const dismiss = async () => {
    if (!selectedId) return;
    setActionError(null);
    try {
      await dismissMutation.mutateAsync(selectedId);
      setSelectedId(null);
    } catch {
      setActionError("Could not dismiss session.");
    }
  };

  const approve = async () => {
    const preview = previewQuery.data;
    if (!selectedId || !preview?.enrolled) return;
    setActionError(null);
    try {
      const result = await approveMutation.mutateAsync(selectedId);
      if (result.approved !== 1) throw new Error("Approval was not accepted");
      onApproved?.({
        kind: "entry",
        id: selectedId,
        hold_until: result.hold_until,
        label: preview.entry.project_label,
      });
      setSelectedId(null);
    } catch {
      setActionError(
        "Could not approve session. Nothing was sent unless Rust accepted approval.",
      );
    }
  };

  const clear = () => {
    setSelectedId(null);
    setActionError(null);
  };

  const state =
    dismissMutation.isPending || approveMutation.isPending
      ? "acting"
      : selectedId === null
        ? "idle"
        : previewQuery.isPending
          ? "loading"
          : previewQuery.isError || actionError
            ? "error"
            : "ready";
  const error = actionError
    ? actionError
    : previewQuery.isError
      ? "The session file changed while it was being read. Nothing has been sent, and nothing will be until it can be shown to you."
      : null;

  return {
    ...previewQuery,
    selectedId,
    preview: previewQuery.data ?? null,
    state: state as "idle" | "loading" | "ready" | "error" | "acting",
    error,
    errorKind: (actionError
      ? "action"
      : previewQuery.isError
        ? "preview"
        : null) as "action" | "preview" | null,
    review,
    dismiss,
    approve,
    clear,
    previewQuery,
    dismissMutation,
    approveMutation,
  };
}
