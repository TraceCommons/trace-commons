import { useMutation, useQueryClient } from "@tanstack/react-query";
import { retryDaemonStartup } from "../lib/tauri/core-api";
import type { CoreStatus } from "../lib/tauri/types";
import { GlassButton, Notice } from "@/design-system";

export function DaemonStartupNotice({ startup }: { startup: CoreStatus["startup"] | undefined }) {
  const queryClient = useQueryClient();
  const retry = useMutation({
    mutationFn: retryDaemonStartup,
    onSuccess: () => queryClient.invalidateQueries(),
  });

  if (startup !== "daemon_unavailable") return null;

  return (
    <Notice tone="ask" title="Rust core could not start">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span>
          Source roots remain saved. Retry when the core is ready to restore live contribution controls.
        </span>
        <GlassButton
          type="button"
          disabled={retry.isPending}
          onClick={() => retry.mutate()}
        >
          {retry.isPending ? "Retrying…" : "Retry core startup"}
        </GlassButton>
        {retry.isError && (
          <span className="w-full text-tc-outside" role="alert">
            Core is still unavailable. Check local setup, then retry.
          </span>
        )}
      </div>
    </Notice>
  );
}
