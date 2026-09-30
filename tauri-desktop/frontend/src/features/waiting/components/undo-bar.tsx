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
    <section className="mb-4 flex items-center justify-between gap-[18px] rounded-2xl border border-border bg-card/80 p-[22px_26px] max-[860px]:items-stretch max-[860px]:flex-col">
      <div>
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
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
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onUndo}
          disabled={busy || !scope}
        >
          {busy ? "Undoing…" : "Undo"}
        </Button>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={onDismiss}
        >
          Dismiss
        </Button>
      </div>
    </section>
  );
}
