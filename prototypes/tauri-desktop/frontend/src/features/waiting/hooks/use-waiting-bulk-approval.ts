import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { waitingKeys } from "../api/query-keys";
import type { UndoScope } from "../api/undo-api";
import { approveWaitingProject } from "../api/waiting-api";

export function useWaitingBulkApproval(
  onApproved?: (scope: UndoScope) => void,
) {
  const [messages, setMessages] = useState<Record<string, string>>({});
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (projectId: string) => approveWaitingProject(projectId),
    onSuccess: async (result, projectId) => {
      const excluded = result.excluded_ineligible
        ? ` ${result.excluded_ineligible} not eligible.`
        : "";
      if (result.approved > 0)
        onApproved?.({
          kind: "project",
          id: projectId,
          hold_until: result.hold_until,
          label: projectId,
        });
      setMessages((current) => ({
        ...current,
        [projectId]: `${result.approved} approved.${excluded}`,
      }));
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
  const approve = async (projectId: string) => {
    setMessages((current) => ({ ...current, [projectId]: "" }));
    try {
      await mutation.mutateAsync(projectId);
    } catch {
      setMessages((current) => ({
        ...current,
        [projectId]: "Could not approve project queue.",
      }));
    }
  };
  return {
    busyId: mutation.isPending ? (mutation.variables ?? null) : null,
    messages,
    approve,
    mutation,
  };
}
