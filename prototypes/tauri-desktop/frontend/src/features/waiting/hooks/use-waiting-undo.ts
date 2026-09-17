import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { coreKeys } from "../../../lib/tauri/query-keys";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { waitingKeys } from "../api/query-keys";
import { cancelApproval, type UndoScope } from "../api/undo-api";

export function useWaitingUndo() {
  const core = useCoreStatus();
  const queryClient = useQueryClient();
  const [scope, setScope] = useState<UndoScope | null>(null);
  const [seconds, setSeconds] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const mutation = useMutation({
    mutationFn: (nextScope: UndoScope) => cancelApproval(nextScope),
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

  useEffect(() => {
    if (!scope?.hold_until) return;
    const tick = () =>
      setSeconds(
        Math.max(
          0,
          Math.ceil(
            (new Date(scope.hold_until as string).getTime() - Date.now()) /
              1000,
          ),
        ),
      );
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [scope]);

  const prepare = useCallback((next: UndoScope) => {
    setScope(next.hold_until ? next : null);
    setError(null);
  }, []);
  const dismiss = useCallback(() => {
    setScope(null);
    setError(null);
  }, []);
  const undo = useCallback(async () => {
    if (!scope || mutation.isPending) return;
    setError(null);
    try {
      await mutation.mutateAsync(scope);
      setScope(null);
    } catch {
      setError(
        "Too late to undo, or approval already left the queue. Check History.",
      );
    }
  }, [mutation, scope]);

  return {
    scope,
    seconds,
    busy: mutation.isPending,
    error,
    prepare,
    dismiss,
    undo,
    mutation,
  };
}
