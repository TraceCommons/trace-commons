import { useState } from "react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";

export function QueueOutcomeDisclosure({
  reasons,
  lines,
}: {
  reasons: Record<string, number> | null;
  lines: Record<string, string>;
}) {
  const [open, setOpen] = useState(false);
  const entries = reasons
    ? Object.entries(reasons).filter(([, count]) => count > 0)
    : [];
  if (entries.length === 0) return null;
  const total = entries.reduce((sum, [, count]) => sum + count, 0);
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="mb-4">
      <div className="rounded-xl border border-border bg-card/80 p-[18px_22px]">
        <CollapsibleTrigger className="flex w-full items-center gap-2 text-left text-[12px] font-bold text-foreground">
          <span aria-hidden="true" className="text-primary">
            {open ? "⌄" : "›"}
          </span>
          Sessions no longer waiting ({total})
        </CollapsibleTrigger>
        <CollapsibleContent className="grid gap-2 pl-5 pt-3 text-[11px] text-muted-foreground">
          {entries
            .sort(([left], [right]) => left.localeCompare(right))
            .map(([label, count]) => (
              <span key={label}>
                {count} — {lines[label] ?? "Status unavailable"}
              </span>
            ))}
          <span className="pt-1 leading-[1.5]">
            This covers sessions that reached the queue. Sessions never queued
            are not counted here.
          </span>
        </CollapsibleContent>
      </div>
    </Collapsible>
  );
}
