import { Button } from "@/components/ui/button";
import type { UndoScope } from "../api/undo-api";

export function UndoBar({
  scope,
  seconds,
  busy,
  error,
  onUndo,
  onDismiss,
}: {
  scope: UndoScope | null;
  seconds: number;
  busy: boolean;
  error: string | null;
  onUndo: () => void;
  onDismiss: () => void;
}) {
  if (!scope && !error) return null;
  return (
    <section className="tc-card mb-4 flex items-center justify-between gap-[18px]">
      <div>
        <span className="mb-1.5 block tc-eyebrow">
          APPROVAL SAVED
        </span>
        <strong>
          {scope ? `${scope.label} approved` : "Undo unavailable"}
        </strong>
        {scope && (
          <small>
            {seconds > 0
              ? `Undo within ${seconds}s before upload starts.`
              : "Upload may already have started."}
          </small>
        )}
        {error && <small>{error}</small>}
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <Button
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={onUndo}
          disabled={busy || !scope}
        >
          {busy ? "Undoing…" : "Undo"}
        </Button>
        <Button
          className="tc-link"
          type="button"
          onClick={onDismiss}
        >
          Dismiss
        </Button>
      </div>
    </section>
  );
}
