import { useMutation, useQueryClient } from "@tanstack/react-query";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { settingsKeys } from "../api/query-keys";
import { setDaemonState } from "../api/settings-api";

export function useDaemonControl() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: setDaemonState,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: coreKeys.status }),
        queryClient.invalidateQueries({
          queryKey: settingsKeys.snapshot(core.scope),
        }),
      ]);
    },
  });

  return {
    busy: mutation.isPending,
    error: mutation.isError ? "Daemon state was not changed." : null,
    command: async (name: "pause_daemon" | "resume_daemon") => {
      try {
        await mutation.mutateAsync(name);
      } catch {
        // Mutation state supplies the panel error.
      }
    },
    mutation,
  };
}
