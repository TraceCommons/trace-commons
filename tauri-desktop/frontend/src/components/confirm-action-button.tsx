import { useState } from "react";
import { ResponsiveOverlay } from "./responsive-overlay";
import { Button } from "./ui/button";

export function ConfirmActionButton({
  label,
  title,
  description,
  confirmLabel,
  cancelLabel,
  workingLabel,
  unavailableLabel,
  busy,
  onConfirm,
}: {
  label: string | undefined;
  title: string | undefined;
  description: string | undefined;
  confirmLabel: string | undefined;
  cancelLabel: string | undefined;
  workingLabel: string | undefined;
  unavailableLabel: string;
  busy: boolean;
  onConfirm: () => void;
}) {
  const [open, setOpen] = useState(false);
  const copyReady = Boolean(label && title && description && confirmLabel && cancelLabel && workingLabel);

  return (
    <>
      <Button
        className="text-destructive"
        type="button"
        variant="outline"
        onClick={() => setOpen(true)}
        disabled={busy || !copyReady}
      >
        {label ?? "Loading…"}
      </Button>
      {!copyReady && (
        <span className="text-xs text-destructive" role="alert">
          {unavailableLabel}
        </span>
      )}
      <ResponsiveOverlay
        open={open}
        onOpenChange={setOpen}
        title={title ?? ""}
        description={description}
        footer={
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={() => setOpen(false)}
            >
              {cancelLabel ?? ""}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={busy || !copyReady}
              onClick={() => {
                setOpen(false);
                onConfirm();
              }}
            >
              {busy ? workingLabel : confirmLabel}
            </Button>
          </div>
        }
      >
        {null}
      </ResponsiveOverlay>
    </>
  );
}
