import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  publishPublicRun,
  unpublishPublicRun,
  validatePublicRunEditor,
} from "../api/public-run-api";
import { historyKeys } from "../api/query-keys";
import type {
  HistoryDetail,
  PublicRunDraft,
  PublicRunEditorInput,
} from "../types";

type PublicRunState =
  | "idle"
  | "reviewing"
  | "publishing"
  | "unpublishing"
  | "error";

export function usePublicRun(
  submissionId: string | null,
  detail: HistoryDetail | null,
) {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<PublicRunDraft | null>(null);
  const [state, setState] = useState<PublicRunState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const validationMutation = useMutation({
    mutationFn: validatePublicRunEditor,
  });
  const publishMutation = useMutation({
    mutationFn: ({
      id,
      nextDraft,
      taskSuccess,
      contributedVersion,
      publicationVersion,
    }: {
      id: string;
      nextDraft: PublicRunDraft;
      taskSuccess: string;
      contributedVersion: string;
      publicationVersion: number;
    }) =>
      publishPublicRun(
        id,
        nextDraft,
        taskSuccess,
        contributedVersion,
        publicationVersion,
      ),
    onSuccess: (_, variables) =>
      Promise.all([
        queryClient.invalidateQueries({
          queryKey: historyKeys.detail(core.scope, variables.id),
        }),
        queryClient.invalidateQueries({
          queryKey: historyKeys.list(core.scope),
        }),
      ]),
  });
  const unpublishMutation = useMutation({
    mutationFn: (id: string) => unpublishPublicRun(id),
    onSuccess: (_, id) =>
      Promise.all([
        queryClient.invalidateQueries({
          queryKey: historyKeys.detail(core.scope, id),
        }),
        queryClient.invalidateQueries({
          queryKey: historyKeys.list(core.scope),
        }),
      ]),
  });

  // biome-ignore lint/correctness/useExhaustiveDependencies: submissionId resets the draft when selection changes before detail identity changes.
  useEffect(() => {
    setDraft(null);
    setError(null);
    setState("idle");
    setEditing(false);
  }, [detail, submissionId]);

  const review = async (input: PublicRunEditorInput) => {
    setState("reviewing");
    setError(null);
    try {
      const result = await validationMutation.mutateAsync(input);
      setDraft(result.draft);
      setError(result.error);
      setState(result.draft ? "idle" : "error");
    } catch {
      setDraft(null);
      setState("error");
      setError("Public page validation unavailable.");
    }
  };

  const publish = async () => {
    if (!submissionId || !detail || !draft || !detail.task_success) return;
    setState("publishing");
    setError(null);
    try {
      await publishMutation.mutateAsync({
        id: submissionId,
        nextDraft: draft,
        taskSuccess: detail.task_success,
        contributedVersion: detail.contributed_version,
        publicationVersion: detail.publication_version,
      });
      setState("idle");
    } catch {
      setState("error");
      setError("Public page could not be changed. Review again and retry.");
    }
  };

  const unpublish = async () => {
    if (!submissionId || !detail) return;
    setState("unpublishing");
    setError(null);
    try {
      await unpublishMutation.mutateAsync(submissionId);
      setState("idle");
    } catch {
      setState("error");
      setError("Public page could not be unpublished. Retry the request.");
    }
  };

  const beginEdit = () => {
    setDraft(null);
    setError(null);
    setEditing(true);
  };

  const cancelEdit = () => {
    setDraft(null);
    setError(null);
    setEditing(false);
  };

  return {
    draft,
    error,
    state,
    editing,
    validationMutation,
    publishMutation,
    unpublishMutation,
    working:
      state === "reviewing" ||
      state === "publishing" ||
      state === "unpublishing",
    review,
    publish,
    unpublish,
    beginEdit,
    cancelEdit,
  };
}
