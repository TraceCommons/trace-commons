import { useState } from "react";
import { ResponsiveOverlay } from "../components/responsive-overlay";
import { Button } from "../components/ui/button";
import { quitApp } from "../lib/tauri/platform-api";
import { useQuitConfirmationCopy } from "../lib/tauri/use-contributor-copy";

export function QuitConfirmation({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  // Hosting, attached, or no watcher: each has its own true sentence, and
  // the Rust core decides which one this process is.
  const copy = useQuitConfirmationCopy(open);
  const confirm = async () => {
    setError(null);
    try {
      await quitApp();
    } catch {
      setError("Trace Commons could not quit. Try again.");
    }
  };
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={onOpenChange}
      title={copy.data?.title ?? "Quit Trace Commons?"}
      description={copy.data?.body}
      footer={
        <>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            {copy.data?.cancel ?? "Cancel"}
          </Button>
          <Button
            type="button"
            variant="destructive"
            // Wait for the true sentence, but never trap the contributor in
            // the app if it cannot be read.
            disabled={!copy.data && !copy.isError}
            onClick={() => void confirm()}
          >
            {copy.data?.confirm ?? "Quit"}
          </Button>
        </>
      }
    >
      {copy.isError && (
        <p className="text-sm text-destructive">
          Trace Commons could not tell whether quitting stops it watching for
          finished sessions. Anything already waiting stays waiting.
        </p>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </ResponsiveOverlay>
  );
}
