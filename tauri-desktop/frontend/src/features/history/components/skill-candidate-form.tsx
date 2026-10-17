import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { type SkillDraftFormValues, skillDraftFormSchema } from "../forms";
import type { SkillCandidate, SkillCopy } from "../skill-types";
import { ButtonPrimary, Input, TextArea } from "@/design-system";

export function SkillCandidateForm({
  candidate,
  copy,
  busy,
  onReview,
}: {
  candidate: SkillCandidate;
  copy: SkillCopy;
  busy: boolean;
  onReview: (draft: SkillDraftFormValues) => void;
}) {
  const form = useForm<SkillDraftFormValues>({
    resolver: zodResolver(skillDraftFormSchema),
    defaultValues: candidate.draft,
    mode: "onChange",
  });
  useEffect(() => {
    if (!form.formState.isDirty) form.reset(candidate.draft);
  }, [candidate.draft, form]);
  const values = form.watch();
  const errors = form.formState.errors;
  return (
    <form
      className="grid gap-4"
      onSubmit={form.handleSubmit((draft) => onReview(draft))}
    >
      <div className="grid gap-[5px] tc-card tc-card--quiet">
        <strong>{copy.generated_source}</strong>
        <span>{candidate.family}</span>
      </div>
      <label>
        {copy.name}
        <Input
          {...form.register("name")}
          maxLength={64}
          disabled={busy}
          aria-invalid={Boolean(errors.name)}
          aria-describedby={errors.name ? "skill-name-error" : undefined}
        />
        <small>{values.name.length}/64 · lowercase hyphenated name</small>
        <FormFieldError id="skill-name-error" message={errors.name?.message} />
      </label>
      <label>
        {copy.applicability}
        <TextArea
          {...form.register("description")}
          maxLength={1024}
          rows={4}
          disabled={busy}
          aria-invalid={Boolean(errors.description)}
          aria-describedby={
            errors.description ? "skill-description-error" : undefined
          }
        />
        <small>{values.description.length}/1024</small>
        <FormFieldError
          id="skill-description-error"
          message={errors.description?.message}
        />
      </label>
      <label>
        {copy.procedure}
        <TextArea
          {...form.register("procedure")}
          maxLength={12000}
          rows={10}
          disabled={busy}
          aria-invalid={Boolean(errors.procedure)}
          aria-describedby={
            errors.procedure ? "skill-procedure-error" : undefined
          }
        />
        <small>{values.procedure.length}/12000</small>
        <FormFieldError
          id="skill-procedure-error"
          message={errors.procedure?.message}
        />
      </label>
      <div className="grid gap-px pt-2">
        <span className="mb-1.5 block tc-eyebrow">
          {copy.source_evidence}
        </span>
        {candidate.source_evidence.map((item) => (
          <div
            className="grid grid-cols-[150px_minmax(0,1fr)] gap-3 border-b border-tc-hairline py-2.5 text-[11px] leading-[1.5] text-tc-secondary max-[860px]:grid-cols-1"
            key={item.event_id}
          >
            <strong>{item.kind}</strong>
            <span>{item.excerpt}</span>
          </div>
        ))}
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        {copy.manual_instruction}
      </p>
      <div className="grid gap-px pt-2">
        <span className="mb-1.5 block tc-eyebrow">
          {copy.test_contract}
        </span>
        <p>
          {copy.contract_summary_format
            .replace("%1$d", String(candidate.evaluation_contract.task_count))
            .replace(
              "%2$d",
              String(candidate.evaluation_contract.cluster_count),
            )
            .replace(
              "%3$d",
              String(candidate.evaluation_contract.total_requests),
            )
            .replace(
              "%4$d",
              String(candidate.evaluation_contract.output_token_limit),
            )
            .replace(
              "%5$d",
              String(candidate.evaluation_contract.request_timeout_seconds),
            )}
        </p>
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <ButtonPrimary size="sm"
          type="submit"
          disabled={busy || !form.formState.isValid}
        >
          {busy ? copy.reviewing : copy.review_action}
        </ButtonPrimary>
      </div>
    </form>
  );
}
