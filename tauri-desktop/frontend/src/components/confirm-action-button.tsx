import { useState } from "react";
import { ResponsiveOverlay } from "./responsive-overlay";
import { GlassButton } from "@/design-system";

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
      <GlassButton
        className="text-tc-outside"
        type="button"
        onClick={() => setOpen(true)}
        disabled={busy || !copyReady}
      >
        {label ?? "Loading…"}
      </GlassButton>
      {!copyReady && (
        <span className="text-xs text-tc-outside" role="alert">
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
            <GlassButton
              type="button"
              onClick={() => setOpen(false)}
            >
              {cancelLabel ?? ""}
            </GlassButton>
            <GlassButton className="tc-text-outside"
              type="button"
              disabled={busy || !copyReady}
              onClick={() => {
                setOpen(false);
                onConfirm();
              }}
            >
              {busy ? workingLabel : confirmLabel}
            </GlassButton>
          </div>
        }
      >
        {null}
      </ResponsiveOverlay>
    </>
  );
}
