import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type { OutcomeCopy } from "../../../lib/tauri/contributor-copy-api";
import type { OutcomeVerdict } from "../types";

export function WaitingOutcomeFields({
  copy,
  verdict,
  correction,
  disabled,
  onVerdictChange,
  onCorrectionChange,
}: {
  copy: OutcomeCopy;
  verdict: OutcomeVerdict | null;
  correction: string;
  disabled: boolean;
  onVerdictChange: (verdict: OutcomeVerdict | null) => void;
  onCorrectionChange: (correction: string) => void;
}) {
  const correctionAllowed = verdict === "partly" || verdict === "failed";
  const verdicts: Array<[OutcomeVerdict, string]> = [
    ["worked", copy.worked],
    ["partly", copy.partly],
    ["failed", copy.failed],
  ];
  return (
    <fieldset className="my-4 grid gap-2 border-0 p-0">
      <legend className="mb-2 text-[12px] font-semibold text-foreground">
        {copy.verdict_question}
      </legend>
      <div className="flex flex-wrap gap-2" role="group" aria-label={copy.verdict_question}>
        {verdicts.map(([value, label]) => (
          <Button
            key={value}
            type="button"
            variant={verdict === value ? "default" : "outline"}
            aria-pressed={verdict === value}
            disabled={disabled}
            onClick={() => onVerdictChange(verdict === value ? null : value)}
          >
            {label}
          </Button>
        ))}
      </div>
      <p className="m-0 text-[11px] leading-[1.5] text-muted-foreground">
        {copy.verdict_caption}
      </p>
      {correctionAllowed && (
        <div className="mt-2 grid gap-2">
          <label
            className="text-[12px] font-semibold text-foreground"
            htmlFor="waiting-correction"
          >
            {copy.correction_question}
          </label>
          <Textarea
            id="waiting-correction"
            value={correction}
            onChange={(event) => onCorrectionChange(event.target.value)}
            placeholder={copy.correction_placeholder}
            maxLength={copy.max_correction_chars}
            disabled={disabled}
            aria-describedby="waiting-correction-caption waiting-correction-count"
          />
          <p
            id="waiting-correction-caption"
            className="m-0 text-[11px] leading-[1.5] text-muted-foreground"
          >
            {copy.correction_caption}
          </p>
          <small
            id="waiting-correction-count"
            className="text-right text-[10px] text-muted-foreground"
          >
            {correction.length}/{copy.max_correction_chars}
          </small>
        </div>
      )}
    </fieldset>
  );
}
