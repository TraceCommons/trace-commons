import { Button } from "@/components/ui/button";
import type { InsightOutcomeLink } from "../types";

type OutcomeLinkListProps = {
  links: InsightOutcomeLink[];
  busy: boolean;
  onUnlink: (id: string) => void;
};

export function OutcomeLinkList({
  links,
  busy,
  onUnlink,
}: OutcomeLinkListProps) {
  if (links.length === 0) {
    return (
      <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
        No outcome evidence linked.
      </p>
    );
  }
  return (
    <div className="mt-3.5 grid gap-2.5">
      {links.map((link) => (
        <article
          className="rounded-[10px] border border-border bg-muted p-3"
          key={link.id}
        >
          <div className="grid grid-cols-[150px_minmax(0,1fr)] gap-3 border-t border-border py-[9px] text-[11px] text-muted-foreground">
            <span>
              {link.evidence.type === "git_commit"
                ? "Inspected Git object"
                : "Imported test report"}
            </span>
            <code>{link.id}</code>
          </div>
          {link.evidence.type === "git_commit" ? (
            <div className="flex flex-wrap gap-x-3.5 gap-y-1.5 text-[10px] text-muted-foreground">
              <span>Object {link.evidence.evidence.object_id}</span>
              <span>Tree {link.evidence.evidence.tree_id}</span>
              <span>
                Parents {link.evidence.evidence.parent_ids.length || "none"}
              </span>
            </div>
          ) : (
            <div className="flex flex-wrap gap-x-3.5 gap-y-1.5 text-[10px] text-muted-foreground">
              <span>Runner {link.evidence.evidence.runner}</span>
              <span>Passed {link.evidence.evidence.passed}</span>
              <span>Failed {link.evidence.evidence.failed}</span>
              <span>Skipped {link.evidence.evidence.skipped}</span>
            </div>
          )}
          <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
            User-linked evidence. It does not prove task success, merge
            acceptance, or test execution here.
          </p>
          <Button
            className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary text-destructive"
            type="button"
            onClick={() => onUnlink(link.id)}
            disabled={busy}
          >
            Unlink evidence
          </Button>
        </article>
      ))}
    </div>
  );
}
