import { useState } from "react";
import { ResponsiveOverlay } from "../components/responsive-overlay";
import { Button } from "../components/ui/button";
import { quitApp } from "../lib/tauri/platform-api";

export function QuitConfirmation({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [error, setError] = useState<string | null>(null);
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
      title="Quit Trace Commons?"
      description="Quitting stops Trace Commons watching for finished sessions. Nothing is queued or sent until you open it again. Sessions already waiting remain on this device for review after relaunch."
      footer={
        <>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Keep open
          </Button>
          <Button type="button" variant="destructive" onClick={() => void confirm()}>
            Quit
          </Button>
        </>
      }
    >
      {error && <p className="text-sm text-destructive">{error}</p>}
    </ResponsiveOverlay>
  );
}
