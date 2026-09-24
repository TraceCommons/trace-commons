import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { waitingKeys } from "../../features/waiting/public";
import { coreKeys } from "../../lib/tauri/query-keys";
import {
  type DaemonEventName,
  useDaemonEvents,
} from "../../lib/tauri/use-daemon-events";

function invalidateForDaemonEvent(
  queryClient: ReturnType<typeof useQueryClient>,
  scope: string,
  event: DaemonEventName,
): void {
  const accountScope = ["account", scope] as const;

  switch (event) {
    case "queue_changed":
      void queryClient.invalidateQueries({
        queryKey: waitingKeys.scope(scope),
      });
      void queryClient.invalidateQueries({ queryKey: coreKeys.status });
      return;
    case "preview_ready":
      void queryClient.invalidateQueries({
        queryKey: waitingKeys.scope(scope),
      });
      return;
    case "status_changed":
    case "snapshot":
    case "resync_required":
      void queryClient.invalidateQueries({ queryKey: accountScope });
      void queryClient.invalidateQueries({ queryKey: coreKeys.status });
      return;
  }
}

export function useDaemonQueryEvents(scope: string): boolean {
  const queryClient = useQueryClient();
  const onEvent = useCallback(
    (event: DaemonEventName) =>
      invalidateForDaemonEvent(queryClient, scope, event),
    [queryClient, scope],
  );

  return useDaemonEvents(onEvent);
}
