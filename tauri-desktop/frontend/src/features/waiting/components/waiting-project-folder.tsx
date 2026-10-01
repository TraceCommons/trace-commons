import type { WaitingEntry } from "../types";
import { useEligibilityGroupCopy } from "../../../lib/tauri/use-contributor-copy";
import { IgnoreProjectControl } from "./ignore-project-control";
import { SubmitAllAsControl } from "./submit-all-as-control";
import { GlassButton } from "@/design-system";
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
    <article className="tc-card tc-card--quiet flex min-w-0 flex-col items-stretch gap-2.5">
      <button
        className="flex min-w-0 border-0 bg-transparent p-0 text-left text-[var(--tc-text-primary)]"
        type="button"
        onClick={() => onOpen(projectId)}
      >
        <span className="min-w-0 break-words">
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
      </button>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        {eligibility.data?.can_contribute === true && (
          <>
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
          <span className="w-full tc-caption tc-text-accent">{message}</span>
        )}
      </div>
    </article>
  );
}
