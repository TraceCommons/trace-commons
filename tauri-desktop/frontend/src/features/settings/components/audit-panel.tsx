import { Button } from "@/components/ui/button";
import type { AuditEntry } from "../api/audit-api";

const actionLabels: Record<string, string> = {
  "armed-auto-upload": "Automatic contributing turned on for",
  "disarmed-auto-upload": "Automatic contributing turned off for",
  "queue-bulk-approved": "The whole queue was approved",
  "consent-scopes-changed": "Permissions changed",
  "near-ai-notice-acknowledged": "The extra privacy scan was confirmed",
};

function sentence(entry: AuditEntry) {
  const label = actionLabels[entry.action] ?? "Changed";
  return entry.project_label ? `${label} ${entry.project_label}` : label;
}

function formatInstant(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf())
    ? value
    : new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(date);
}

export function AuditPanel({
  entries,
  state,
  error,
  onRefresh,
}: {
  entries: AuditEntry[];
  state: "loading" | "ready" | "error";
  error: string | null;
  onRefresh: () => Promise<void>;
}) {
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            LOCAL VISIBILITY
          </span>
          <h2>Recent changes</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={state === "loading"}
        >
          Refresh
        </Button>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Visibility record only. It does not authorize, block, or undo changes.
      </p>
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Reading recent changes…
        </p>
      )}
      {state === "error" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Audit log unavailable.
        </p>
      )}
      {state === "ready" && entries.length === 0 && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Nothing has been changed.
        </p>
      )}
      {state === "ready" && entries.length > 0 && (
        <div className="mt-[18px] grid gap-px border-t border-border">
          {entries.map((entry) => (
            <div
              className="grid grid-cols-[170px_minmax(0,1fr)] gap-4 border-b border-border py-[13px] text-[11px] leading-[1.45] text-muted-foreground"
              key={`${entry.at}-${entry.action}-${entry.project_label ?? "global"}-${entry.detail ?? ""}`}
            >
              <time dateTime={entry.at}>{formatInstant(entry.at)}</time>
              <span>{sentence(entry)}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
