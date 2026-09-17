import { Button } from "@/components/ui/button";
import type { WaitingEntry } from "../types";
import { WaitingEntryRow } from "./waiting-entry-row";

export function WaitingProjectGroup({
  projectId,
  label,
  entries,
  selectedId,
  busy,
  message,
  onReview,
  onSubmitAll,
  showSubmitAll = true,
}: {
  projectId: string;
  label: string;
  entries: WaitingEntry[];
  selectedId: string | null;
  busy: boolean;
  message?: string;
  onReview: (entryId: string) => void;
  onSubmitAll: (projectId: string) => void;
  showSubmitAll?: boolean;
}) {
  return (
    <section className="border-t border-border py-[18px] first:border-t-0 first:pt-0">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            PROJECT
          </span>
          <h3>{label}</h3>
          <span>
            {entries.length} waiting session{entries.length === 1 ? "" : "s"}
          </span>
        </div>
        {showSubmitAll && (
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={() => onSubmitAll(projectId)}
            disabled={busy}
          >
            {busy ? "Submitting…" : "Submit all"}
          </Button>
        )}
      </div>
      {message && (
        <p className="mt-[15px] text-[11px] leading-[1.5] text-primary">
          {message}
        </p>
      )}
      <div className="mt-[22px] grid gap-px border-t border-border">
        {entries.map((entry) => (
          <WaitingEntryRow
            key={entry.entry_id}
            entry={entry}
            selected={selectedId === entry.entry_id}
            onReview={() => onReview(entry.entry_id)}
          />
        ))}
      </div>
    </section>
  );
}
