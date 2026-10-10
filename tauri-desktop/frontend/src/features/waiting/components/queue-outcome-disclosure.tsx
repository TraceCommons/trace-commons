import { useId, useState } from "react";
import { Expander } from "@/design-system";

export function QueueOutcomeDisclosure({
  reasons,
  lines,
}: {
  reasons: Record<string, number> | null;
  lines: Record<string, string>;
}) {
  const [open, setOpen] = useState(false);
  const contentId = useId();
  const entries = reasons
    ? Object.entries(reasons).filter(([, count]) => count > 0)
    : [];
  if (entries.length === 0) return null;
  const total = entries.reduce((sum, [, count]) => sum + count, 0);
  return (
    <div className="mb-4 tc-card tc-card--quiet">
      <Expander
        open={open}
        onToggle={() => setOpen((current) => !current)}
        controls={contentId}
      >
        Traces no longer waiting ({total})
      </Expander>
      {open && (
        <div
          id={contentId}
          className="grid gap-2 pl-5 pt-3 text-[11px] text-tc-secondary"
        >
          {entries
            .sort(([left], [right]) => left.localeCompare(right))
            .map(([label, count]) => (
              <span key={label}>
                {count} — {lines[label] ?? "Status unavailable"}
              </span>
            ))}
          <span className="pt-1 leading-[1.5]">
            This covers traces that reached the queue. Traces never queued
            are not counted here.
          </span>
        </div>
      )}
    </div>
  );
}
