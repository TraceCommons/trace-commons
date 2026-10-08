import { useState } from "react";
import { ResponsiveOverlay } from "../components/responsive-overlay";
import { Button } from "../components/ui/button";
import { quitApp } from "../lib/tauri/platform-api";
import { QUIT_FALLBACK, WORDING_UNREADABLE } from "../lib/copy-unreadable";
import {
  useQuitConfirmationCopy,
  useShellStatusLines,
} from "../lib/tauri/use-contributor-copy";

export function QuitConfirmation({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [failed, setFailed] = useState(false);
  const lines = useShellStatusLines();
  // Hosting, attached, or no watcher: each has its own true sentence, and
  // the Rust core decides which one this process is.
  const copy = useQuitConfirmationCopy(open);
  const confirm = async () => {
    setFailed(false);
    try {
      await quitApp();
    } catch {
      setFailed(true);
    }
  };
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title={copy.data?.title ?? QUIT_FALLBACK.title}
      description={copy.data?.body}
      footer={
        <>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            {copy.data?.cancel ?? QUIT_FALLBACK.cancel}
          </Button>
          <Button
            type="button"
            variant="destructive"
            // Wait for the true sentence, but never trap the contributor in
            // the app if it cannot be read.
            disabled={!copy.data && !copy.isError}
            onClick={() => void confirm()}
          >
            {copy.data?.confirm ?? QUIT_FALLBACK.confirm}
          </Button>
        </>
      }
    >
      {/* The core's sentence for this process is what failed to arrive, so
          the shell says only that, and claims nothing about what keeps
          running. */}
      {copy.isError && (
        <p className="text-sm text-destructive">{WORDING_UNREADABLE}</p>
      )}
      {failed && (
        <p className="text-sm text-destructive">{lines.requestFailed}</p>
      )}
    </ResponsiveOverlay>
  );
}
