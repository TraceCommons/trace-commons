import { Button } from "@/components/ui/button";
import type { WaitingEntry } from "../types";
import { useEligibilityGroupCopy } from "../../../lib/tauri/use-contributor-copy";
import { IgnoreProjectControl } from "./ignore-project-control";
import { SubmitAllAsControl } from "./submit-all-as-control";
export function WaitingProjectFolder({
  projectId,
  label,
  path,
  count,
  entries,
  busy,
  message,
  onOpen,
  onSubmitAll,
  onSubmitAllAs,
  onIgnore,
}: {
  projectId: string;
  label: string;
  path?: string;
  count: number;
  entries: WaitingEntry[];
  busy: boolean;
  message?: string;
  onOpen: (projectId: string) => void;
  onSubmitAll: (projectId: string) => void;
  onSubmitAllAs: (projectId: string, outcome: "worked" | "partly" | "failed") => void;
  onIgnore: (projectId: string) => Promise<unknown>;
}) {
  const eligibility = useEligibilityGroupCopy(entries);
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
          <small>{count} waiting sessions</small>
          {eligibility.data && (
            <small>
              {eligibility.data.eligible_count} eligible
              {eligibility.data.withheld_line
                ? ` · ${eligibility.data.withheld_line}`
                : ""}
            </small>
          )}
          {eligibility.isPending && <small>Checking contribution eligibility…</small>}
          {eligibility.isError && (
            <small>Eligibility unavailable. Review sessions individually.</small>
          )}
        </span>
      </Button>
      <div className="flex flex-wrap items-center justify-end gap-2.5">
        {eligibility.data?.can_contribute === true && (
          <>
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
          </>
        )}
        <IgnoreProjectControl
          projectId={projectId}
          label={label}
          pending={count}
          disabled={busy || eligibility.isPending}
          onIgnore={onIgnore}
        />
        {message && (
          <span className="mt-[15px] text-[11px] leading-[1.5] text-primary">
            {message}
          </span>
        )}
      </div>
    </article>
  );
}
