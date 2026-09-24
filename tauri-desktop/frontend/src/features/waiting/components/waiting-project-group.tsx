import { Button } from "@/components/ui/button";
import type { WaitingEntry } from "../types";
import { WaitingEntryRow } from "./waiting-entry-row";
import { useEligibilityGroupCopy } from "../../../lib/tauri/use-contributor-copy";
import { SubmitAllAsControl } from "./submit-all-as-control";

export function WaitingProjectGroup({
  projectId,
  label,
  entries,
  selectedId,
  busy,
  message,
  onReview,
  onSubmitAll,
  onSubmitAllAs,
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
  onSubmitAllAs: (projectId: string, outcome: "worked" | "partly" | "failed") => void;
  showSubmitAll?: boolean;
}) {
  const eligibility = useEligibilityGroupCopy(entries);
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
          {eligibility.data && (
            <p className="m-0 mt-1 text-sm text-muted-foreground">
              {eligibility.data.eligible_count} eligible
              {eligibility.data.withheld_line
                ? ` · ${eligibility.data.withheld_line}`
                : ""}
            </p>
          )}
          {eligibility.isError && (
            <p className="m-0 mt-1 text-sm text-destructive">
              Eligibility unavailable. Review sessions individually.
            </p>
          )}
        </div>
        {showSubmitAll && eligibility.data?.can_contribute === true && (
          <div className="flex flex-wrap gap-2">
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={() => onSubmitAll(projectId)}
              disabled={busy}
            >
              {busy
                ? "Submitting…"
                : `Submit all eligible (${eligibility.data.eligible_count})`}
            </Button>
            <SubmitAllAsControl
              eligibleCount={eligibility.data.eligible_count}
              busy={busy}
              onSubmit={(outcome) => onSubmitAllAs(projectId, outcome)}
            />
          </div>
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
