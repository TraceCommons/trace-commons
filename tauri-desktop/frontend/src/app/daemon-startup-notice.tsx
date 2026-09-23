import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import { Button } from "../components/ui/button";
import { retryDaemonStartup } from "../lib/tauri/core-api";
import type { CoreStatus } from "../lib/tauri/types";

export function DaemonStartupNotice({ startup }: { startup: CoreStatus["startup"] | undefined }) {
  const queryClient = useQueryClient();
  const retry = useMutation({
    mutationFn: retryDaemonStartup,
    onSuccess: () => queryClient.invalidateQueries(),
  });

  if (startup !== "daemon_unavailable") return null;

  return (
    <Alert className="mx-6 mt-4 border-amber-500/40 bg-amber-500/10">
      <AlertTitle>Rust core could not start</AlertTitle>
      <AlertDescription className="flex flex-wrap items-center justify-between gap-3">
        <span>
          Source roots remain saved. Retry when the core is ready to restore live contribution controls.
        </span>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={retry.isPending}
          onClick={() => retry.mutate()}
        >
          {retry.isPending ? "Retrying…" : "Retry core startup"}
        </Button>
        {retry.isError && (
          <span className="w-full text-destructive" role="alert">
            Core is still unavailable. Check local setup, then retry.
          </span>
        )}
      </AlertDescription>
    </Alert>
  );
}
