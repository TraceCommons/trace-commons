import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  type ArmingOffer,
  acceptArming,
  declineArming,
  getArmingSuggestion,
} from "../api/arming-api";
import { waitingKeys } from "../api/query-keys";

export function useArmingOffer() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const query = useQuery<ArmingOffer | null>({
    queryKey: waitingKeys.arming(core.scope),
    queryFn: getArmingSuggestion,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: ({
      projectId,
      action,
    }: {
      projectId: string;
      action: "accept" | "decline";
    }) =>
      action === "accept" ? acceptArming(projectId) : declineArming(projectId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: waitingKeys.arming(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.list(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
      ]);
    },
  });
  const act = async (action: "accept" | "decline") => {
    const projectId = query.data?.project_id;
    if (!projectId) return;
    try {
      await mutation.mutateAsync({ projectId, action });
    } catch {
      // Derived mutation state supplies current error copy.
    }
  };
  return {
    ...query,
    offer: query.data ?? null,
    state: mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError || mutation.isError
          ? "error"
          : "ready",
    error: query.isError
      ? "Automatic-contribution suggestion unavailable."
      : mutation.isError
        ? "Automatic-contribution choice was not changed."
        : null,
    refresh: async () => {
      await query.refetch();
    },
    accept: () => act("accept"),
    decline: () => act("decline"),
    mutation,
  };
}
