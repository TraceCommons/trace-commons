import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { settingsKeys } from "../../settings/public";
import {
  answerPrivateInferenceOffer,
  getPrivateInferenceOfferState,
} from "../api/private-inference-offer-api";
import { waitingKeys } from "../api/query-keys";

export function usePrivateInferenceOffer() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const disclosure = useContributorDisclosureCopy();
  const query = useQuery({
    queryKey: waitingKeys.privateInference(core.scope),
    queryFn: getPrivateInferenceOfferState,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: answerPrivateInferenceOffer,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: waitingKeys.privateInference(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.list(core.scope),
        }),
        queryClient.invalidateQueries({
          queryKey: waitingKeys.arming(core.scope),
        }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });
  const answer = async (enabled: boolean) => {
    try {
      await mutation.mutateAsync(enabled);
    } catch {
      // Derived mutation state supplies current error copy.
    }
  };
  return {
    ...query,
    offered: query.data
      ? query.data.configured && !query.data.answered && !query.data.enabled
      : false,
    busy: mutation.isPending,
    // A failed write may still have persisted, so the shared sentence says
    // the change is unconfirmed rather than claiming nothing changed.
    error: mutation.isError
      ? (disclosure.data?.private_inference.write_unconfirmed ??
        "The change could not be confirmed.")
      : null,
    refresh: async () => {
      await query.refetch();
    },
    answer,
    mutation,
  };
}
