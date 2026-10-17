import type { AuditEntry } from "../api/audit-api";
import { TertiaryLink } from "@/design-system";

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
    <section className="tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            LOCAL VISIBILITY
          </span>
          <h2>Recent changes</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={state === "loading"}
        >
          Refresh
        </TertiaryLink>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Visibility record only. It does not authorize, block, or undo changes.
      </p>
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Reading recent changes…
        </p>
      )}
      {state === "error" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Audit log unavailable.
        </p>
      )}
      {state === "ready" && entries.length === 0 && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Nothing has been changed.
        </p>
      )}
      {state === "ready" && entries.length > 0 && (
        <div className="mt-[18px] grid gap-px border-t border-tc-hairline">
          {entries.map((entry) => (
            <div
              className="grid grid-cols-[170px_minmax(0,1fr)] gap-4 border-b border-tc-hairline py-[13px] text-[11px] leading-[1.45] text-tc-secondary"
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
