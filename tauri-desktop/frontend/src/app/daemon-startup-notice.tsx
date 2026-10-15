import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Alert, AlertDescription, AlertTitle } from "../components/ui/alert";
import { Button } from "../components/ui/button";
import { retryDaemonStartup } from "../lib/tauri/core-api";
import type { CoreStatus } from "../lib/tauri/types";
import { useShellStatusLines } from "../lib/tauri/use-contributor-copy";

// The core's core-down banner (`health_copy::core_down_copy`), and its
// start-again button. If the core's words cannot be read, the banner says
// only that, in the shell's one sentence, and offers no button it has no
// words for.
export function DaemonStartupNotice({ startup }: { startup: CoreStatus["startup"] | undefined }) {
  const queryClient = useQueryClient();
  const lines = useShellStatusLines();
  const retry = useMutation({
    mutationFn: retryDaemonStartup,
    onSuccess: () => queryClient.invalidateQueries(),
  });

  if (startup !== "daemon_unavailable") return null;

  return (
    <Alert className="mx-6 mt-4 border-amber-500/40 bg-amber-500/10">
      <AlertTitle>{lines.coreDown.title}</AlertTitle>
      <AlertDescription className="flex flex-wrap items-center justify-between gap-3">
        <span>{lines.coreDown.detail}</span>
        {lines.retryStartup && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={retry.isPending}
            onClick={() => retry.mutate()}
          >
            {retry.isPending ? lines.retrying : lines.retryStartup}
          </Button>
        )}
        {retry.isError && (
          <span className="w-full text-destructive" role="alert">
            {lines.requestFailed}
          </span>
        )}
      </AlertDescription>
    </Alert>
  );
}
