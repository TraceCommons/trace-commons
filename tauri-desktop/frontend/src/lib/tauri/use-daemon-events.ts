import { useEffect, useState } from "react";
import { listenTauri } from "./core-api";

const daemonEventNames = [
  "snapshot",
  "queue_changed",
  "status_changed",
  "resync_required",
  "preview_ready",
] as const;

export type DaemonEventName = (typeof daemonEventNames)[number];

function isDaemonEventName(value: string): value is DaemonEventName {
  return daemonEventNames.some((event) => event === value);
}

function parseDaemonEvent(value: unknown): DaemonEventName | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const event = (value as Record<string, unknown>).event;
  return typeof event === "string" && isDaemonEventName(event) ? event : null;
}

export function useDaemonEvents(
  onEvent: (event: DaemonEventName) => void,
): boolean {
  const [hasError, setHasError] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let cleanup: (() => void) | null = null;

    const subscribe = async (): Promise<void> => {
      cleanup = await listenTauri("daemon-event", (payload) => {
        if (cancelled) return;
        const event = parseDaemonEvent(payload);
        if (event) onEvent(event);
      });
      if (cancelled) cleanup();
      else setHasError(false);
    };

    void subscribe().catch(() => {
      if (!cancelled) setHasError(true);
    });

    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, [onEvent]);

  return hasError;
}
