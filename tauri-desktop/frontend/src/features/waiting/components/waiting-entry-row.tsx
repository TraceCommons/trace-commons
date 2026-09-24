import { Button } from "@/components/ui/button";
import { useEligibilityCopy } from "../../../lib/tauri/use-contributor-copy";
import type { WaitingEntry } from "../types";

export function WaitingEntryRow({
  entry,
  selected = false,
  onReview = () => undefined,
}: {
  entry: WaitingEntry;
  selected?: boolean;
  onReview?: () => void;
}) {
  const eligibility = useEligibilityCopy(
    entry.eligibility,
    entry.eligibility_reason,
  );
  const size = `${Math.max(1, Math.round(entry.size_bytes / 1024))} KB`;
  const detail =
    entry.subagent_count > 0
      ? `${entry.subagent_count} delegated transcript${entry.subagent_count === 1 ? "" : "s"}`
      : "Single session";
  const proof = entry.holds_certificate ? "Witness certificate held" : null;
  const attestationTone =
    entry.attestation_copy?.tone === "clear"
      ? "text-green"
      : entry.attestation_copy?.tone === "attention"
        ? "text-gold"
        : entry.attestation_copy?.tone === "refused"
          ? "text-destructive"
          : entry.attestation_copy?.tone === "held"
            ? "text-blue"
            : "text-muted-foreground";
  return (
    <article
      className={`grid grid-cols-[38px_minmax(0,1fr)_auto_auto] items-center gap-3.5 border-b border-border py-3.5 max-[860px]:grid-cols-[38px_minmax(0,1fr)_auto] ${selected ? "bg-primary/10 font-bold text-primary" : ""}`}
    >
      <div className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground">
        {entry.project_label.slice(0, 1).toUpperCase()}
      </div>
      <div className="grid min-w-0 gap-1">
        <strong>{entry.project_label}</strong>
        <span>
          {entry.source} · {detail}
        </span>
        {proof && <small className="text-[10px] text-muted-foreground">{proof}</small>}
        {entry.attestation_copy && (
          <div className={`grid gap-0.5 text-[10px] leading-[1.45] ${attestationTone}`}>
            <small>{entry.attestation_copy.state_line}</small>
            {entry.attestation_copy.reason_line && (
              <small className="text-muted-foreground">
                {entry.attestation_copy.reason_line}
              </small>
            )}
          </div>
        )}
        {entry.eligibility && eligibility.data && (
          <div className="grid gap-0.5 text-[10px] leading-[1.45] text-muted-foreground">
            <small>{eligibility.data.state_line}</small>
            {eligibility.data.reason_line && (
              <small>{eligibility.data.reason_line}</small>
            )}
          </div>
        )}
        {entry.eligibility && eligibility.isError && (
          <small className="text-[10px] text-destructive">
            Eligibility details unavailable.
          </small>
        )}
        {entry.session_path && <small>{entry.session_path}</small>}
      </div>
      <div className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
        <strong>{size}</strong>
        <span>
          {entry.subagents_dropped > 0 ? "Trimmed" : "Ready for review"}
        </span>
      </div>
      <Button
        className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
        type="button"
        onClick={onReview}
      >
        {selected ? "Selected" : "Review"}
      </Button>
    </article>
  );
}
