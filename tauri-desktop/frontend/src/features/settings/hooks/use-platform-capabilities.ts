import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getPlatformCapabilities,
  requestNotificationPermission,
  setStartAtLogin,
} from "../../../lib/tauri/platform-api";
import { useShellStatusLines } from "../../../lib/tauri/use-contributor-copy";

const platformKey = ["local", "platform", "capabilities"] as const;

export function usePlatformCapabilities() {
  const queryClient = useQueryClient();
  const lines = useShellStatusLines();
  const query = useQuery({
    queryKey: platformKey,
    queryFn: getPlatformCapabilities,
    staleTime: 60_000,
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey: platformKey });
  const notification = useMutation({
    mutationFn: requestNotificationPermission,
    onSuccess: refresh,
  });
  const login = useMutation({
    mutationFn: setStartAtLogin,
    onSuccess: refresh,
  });
  return {
    ...query,
    data: query.data ?? null,
    state: query.isPending ? "loading" : query.isError ? "error" : "ready",
    busy: notification.isPending || login.isPending,
    requestNotifications: notification.mutateAsync,
    setStartAtLogin: login.mutateAsync,
    // The core's line, never the raw error: it can carry a URL or a path.
    error: query.isError
      ? lines.readUnavailable
      : notification.isError || login.isError
        ? lines.requestFailed
        : null,
  };
}
