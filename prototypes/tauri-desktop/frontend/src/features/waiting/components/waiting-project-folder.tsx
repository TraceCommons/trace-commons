import { Button } from "@/components/ui/button";
export function WaitingProjectFolder({
  projectId,
  label,
  path,
  count,
  busy,
  message,
  onOpen,
  onSubmitAll,
}: {
  projectId: string;
  label: string;
  path?: string;
  count: number;
  busy: boolean;
  message?: string;
  onOpen: (projectId: string) => void;
  onSubmitAll: (projectId: string) => void;
}) {
  return (
    <article className="flex items-center justify-between gap-[18px] rounded-xl border border-border bg-background p-4">
      <Button
        className="flex min-w-0 items-center gap-3 border-0 bg-transparent p-0 text-left text-foreground"
        type="button"
        onClick={() => onOpen(projectId)}
      >
        <span className="grid h-[38px] w-[38px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground">
          {label.slice(0, 1).toUpperCase()}
        </span>
        <span>
          <strong>{label}</strong>
          {path && <small>{path}</small>}
          <small>
            {count} waiting session{count === 1 ? "" : "s"}
          </small>
        </span>
      </Button>
      <div className="flex flex-wrap items-center justify-end gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={() => onSubmitAll(projectId)}
          disabled={busy}
        >
          {busy ? "Submitting…" : "Submit all"}
        </Button>
        {message && (
          <span className="mt-[15px] text-[11px] leading-[1.5] text-primary">
            {message}
          </span>
        )}
      </div>
    </article>
  );
}
