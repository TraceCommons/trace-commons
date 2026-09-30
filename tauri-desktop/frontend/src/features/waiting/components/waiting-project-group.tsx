import type { WaitingEntry } from "../types";
import { WaitingEntryRow } from "./waiting-entry-row";
import { useEligibilityGroupCopy } from "../../../lib/tauri/use-contributor-copy";
import { SubmitAllAsControl } from "./submit-all-as-control";
import { GlassButton } from "@/design-system";

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
    <section className="border-t border-tc-hairline py-[18px] first:border-t-0 first:pt-0">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            PROJECT
          </span>
          <h3>{label}</h3>
          <span>
            {entries.length} waiting session{entries.length === 1 ? "" : "s"}
          </span>
          {eligibility.data && (
            <p className="m-0 mt-1 text-sm text-tc-secondary">
              {eligibility.data.eligible_count} eligible
              {eligibility.data.withheld_line
                ? ` · ${eligibility.data.withheld_line}`
                : ""}
            </p>
          )}
          {eligibility.isError && (
            <p className="m-0 mt-1 text-sm text-tc-outside">
              Eligibility unavailable. Review sessions individually.
            </p>
          )}
        </div>
        {showSubmitAll && eligibility.data?.can_contribute === true && (
          <div className="flex flex-wrap gap-2">
            <GlassButton
              type="button"
              onClick={() => onSubmitAll(projectId)}
              disabled={busy}
            >
              {busy
                ? "Submitting…"
                : `Submit all eligible (${eligibility.data.eligible_count})`}
            </GlassButton>
            <SubmitAllAsControl
              eligibleCount={eligibility.data.eligible_count}
              busy={busy}
              onSubmit={(outcome) => onSubmitAllAs(projectId, outcome)}
            />
          </div>
        )}
      </div>
      {message && (
        <p className="mt-2 tc-caption tc-text-accent">
          {message}
        </p>
      )}
      <div className="mt-3 grid gap-px">
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
