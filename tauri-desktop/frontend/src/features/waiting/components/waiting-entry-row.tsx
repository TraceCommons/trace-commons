import { useEligibilityCopy } from "../../../lib/tauri/use-contributor-copy";
import type { WaitingEntry } from "../types";
import { GlassButton } from "@/design-system";

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
      : "Single trace";
  const proof = entry.holds_certificate ? "Witness certificate held" : null;
  const attestationTone =
    entry.attestation_copy?.tone === "clear"
      ? "text-tc-accent"
      : entry.attestation_copy?.tone === "attention"
        ? "text-tc-ask"
        : entry.attestation_copy?.tone === "refused"
          ? "text-tc-outside"
          : entry.attestation_copy?.tone === "held"
            ? "text-tc-blue"
            : "text-tc-secondary";
  return (
    <article
      className={`grid grid-cols-[38px_minmax(0,1fr)_auto_auto] items-center gap-3.5 border-b border-tc-hairline py-3.5 max-[860px]:grid-cols-[38px_minmax(0,1fr)_auto] ${selected ? "bg-tc-purple/10 font-bold text-tc-accent" : ""}`}
    >
      <div className="tc-tool-tile tc-tool-tile--lg">
        {entry.project_label.slice(0, 1).toUpperCase()}
      </div>
      <div className="grid min-w-0 gap-1">
        <strong>{entry.project_label}</strong>
        <span>
          {entry.source} · {detail}
        </span>
        {proof && <small className="text-[10px] text-tc-secondary">{proof}</small>}
        {entry.attestation_copy && (
          <div className={`grid gap-0.5 text-[10px] leading-[1.45] ${attestationTone}`}>
            <small>{entry.attestation_copy.state_line}</small>
            {entry.attestation_copy.reason_line && (
              <small className="text-tc-secondary">
                {entry.attestation_copy.reason_line}
              </small>
            )}
          </div>
        )}
        {entry.eligibility && eligibility.data && (
          <div className="grid gap-0.5 text-[10px] leading-[1.45] text-tc-secondary">
            <small>{eligibility.data.state_line}</small>
            {eligibility.data.reason_line && (
              <small>{eligibility.data.reason_line}</small>
            )}
          </div>
        )}
        {entry.eligibility && eligibility.isError && (
          <small className="text-[10px] text-tc-outside">
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
      <GlassButton
        type="button"
        onClick={onReview}
      >
        {selected ? "Selected" : "Review"}
      </GlassButton>
    </article>
  );
}
