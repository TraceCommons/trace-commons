import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useProjectIgnoreCopy } from "../../../lib/tauri/use-contributor-copy";

export function IgnoreProjectControl({
  projectId,
  label,
  pending,
  disabled,
  onIgnore,
}: {
  projectId: string;
  label: string;
  pending: number;
  disabled: boolean;
  onIgnore: (projectId: string) => Promise<unknown>;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const copy = useProjectIgnoreCopy(label, pending);
  const confirm = async () => {
    setBusy(true);
    setError(null);
    try {
      await onIgnore(projectId);
      setOpen(false);
    } catch {
      setError("Project could not be ignored. The queue was not changed.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button
        type="button"
        variant="outline"
        title={copy.data?.tooltip}
        onClick={() => setOpen(true)}
        disabled={disabled || !copy.data}
      >
        {copy.data?.button ?? "Ignore project"}
      </Button>
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title={copy.data?.title ?? "Ignore project?"}
        description={copy.data?.tooltip}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setOpen(false)}
              disabled={busy}
            >
              Keep project
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => void confirm()}
              disabled={busy || !copy.data}
            >
              {busy ? "Ignoring…" : (copy.data?.button ?? "Ignore project")}
            </Button>
          </div>
        }
      >
        {copy.data ? (
          <p className="whitespace-pre-line text-sm leading-[1.55] text-muted-foreground">
            {copy.data.body}
          </p>
        ) : (
          <p className="text-sm text-destructive">
            {copy.isError
              ? "Ignore confirmation unavailable. Ignoring is disabled."
              : "Loading ignore confirmation…"}
          </p>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </ResponsiveOverlay>
    </>
  );
}
