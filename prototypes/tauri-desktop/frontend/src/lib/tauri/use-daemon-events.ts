import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { listenTauri } from "./core-api";

export function useDaemonEvents() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let cancelled = false;
    let cleanup: (() => void) | null = null;
    const subscribe = async () => {
      cleanup = await listenTauri<unknown>("daemon-event", () => {
        if (!cancelled) void queryClient.invalidateQueries();
      });
      if (cancelled) cleanup();
    };
    void subscribe();
    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, [queryClient]);
}
