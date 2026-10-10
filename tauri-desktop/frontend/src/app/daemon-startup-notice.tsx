import { useMutation, useQueryClient } from "@tanstack/react-query";
import { retryDaemonStartup } from "../lib/tauri/core-api";
import type { CoreStatus } from "../lib/tauri/types";
import { GlassButton, Notice } from "@/design-system";
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
    <Notice tone="ask" title={lines.coreDown.title}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span>{lines.coreDown.detail}</span>
        {lines.retryStartup && (
          <GlassButton
            type="button"
            disabled={retry.isPending}
            onClick={() => retry.mutate()}
          >
            {retry.isPending ? lines.retrying : lines.retryStartup}
          </GlassButton>
        )}
        {retry.isError && (
          <span className="w-full text-tc-outside" role="alert">
            {lines.requestFailed}
          </span>
        )}
      </div>
    </Notice>
  );
}
