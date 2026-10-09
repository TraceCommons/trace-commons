import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useShellStatusLines } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import {
  type AutomaticGrant,
  getAutomaticGrant,
  onboardingKeys,
  withdrawAndConfirm,
  withdrawAutomaticGrant,
} from "../../onboarding/public";

// Whether the Flow 1 grant is in force, from the daemon (`automatic_grant`),
// and the way to withdraw it after onboarding. A withdraw is confirmed by
// re-reading that status, never by the withdraw call's own answer.
export function useAutomaticGrant() {
  const lines = useShellStatusLines();
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const queryKey = onboardingKeys.automaticGrant(core.scope);
  const query = useQuery<AutomaticGrant>({
    queryKey,
    queryFn: getAutomaticGrant,
    enabled: core.isSuccess,
  });
  const mutation = useMutation({
    mutationFn: () =>
      withdrawAndConfirm(withdrawAutomaticGrant, async () => {
        const grant = await getAutomaticGrant();
        queryClient.setQueryData(queryKey, grant);
        return grant;
      }),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey }),
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
      ]);
    },
  });
  return {
    grant: query.data ?? null,
    state: (mutation.isPending
      ? "busy"
      : query.isPending
        ? "loading"
        : query.isError
          ? "error"
          : "ready") as "busy" | "loading" | "ready" | "error",
    error: query.isError
      ? lines.readUnavailable
      : mutation.isError
        ? "Automatic contributing was not turned off. Nothing changed."
        : mutation.data === "still_granted"
          ? "Automatic contributing is still on. Try again."
          : null,
    withdrawn: mutation.data === "withdrawn",
    refresh: async () => {
      mutation.reset();
      await query.refetch();
    },
    withdraw: async () => {
      try {
        await mutation.mutateAsync();
      } catch {
        // The mutation state supplies the error copy.
      }
    },
  };
}
