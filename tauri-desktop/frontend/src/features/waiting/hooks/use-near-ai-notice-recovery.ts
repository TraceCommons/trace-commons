import { useMutation, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { acknowledgeNearAiNotice } from "../../onboarding/public";
import { waitingKeys } from "../api/query-keys";

export function useNearAiNoticeRecovery() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: acknowledgeNearAiNotice,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.scope(core.scope),
        }),
      ]);
    },
  });
}
