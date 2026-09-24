import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useController, useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import {
  type PublicRunFormValues,
  publicRunFormSchema,
  publicRunInputFromDetail,
} from "../forms";
import type {
  HistoryDetail,
  PublicRunEditorInput,
  PublicRunReusePermission,
} from "../types";

type Props = {
  detail: HistoryDetail;
  working: boolean;
  error: string | null;
  editingPublished: boolean;
  onReview: (input: PublicRunEditorInput) => void;
  onCancel: () => void;
};

const permissions: Array<{
  value: PublicRunReusePermission;
  label: string;
  detail: string;
}> = [
  {
    value: "cc_by_4_0",
    label: "CC BY 4.0",
    detail: "Others may reuse it with attribution.",
  },
  {
    value: "cc0_1_0",
    label: "CC0 1.0",
    detail: "Others may reuse it without attribution.",
  },
];

export function PublicRunForm({
  detail,
  working,
  error,
  editingPublished,
  onReview,
  onCancel,
}: Props) {
  const correctionAvailable = detail.human_correction !== null;
  const form = useForm<PublicRunFormValues>({
    resolver: zodResolver(publicRunFormSchema),
    defaultValues: publicRunInputFromDetail(detail),
    mode: "onChange",
  });
  const evidence = useController({ control: form.control, name: "evidence" });
  const correction = useController({
    control: form.control,
    name: "correction_excerpt",
  });
  const reusePermission = useController({
    control: form.control,
    name: "reuse_permission",
  });
  useEffect(() => {
    if (!form.formState.isDirty) form.reset(publicRunInputFromDetail(detail));
  }, [detail, form]);
  const title = form.watch("title");
  const outcomeSummary = form.watch("outcome_summary");
  const workflow = form.watch("workflow");
  const errors = form.formState.errors;
  const toggleEvidence = (eventId: string, excerpt: string) => {
    const selected = evidence.field.value.some(
      (item) => item.event_id === eventId,
    );
    evidence.field.onChange(
      selected
        ? evidence.field.value.filter((item) => item.event_id !== eventId)
        : [...evidence.field.value, { event_id: eventId, excerpt }],
    );
  };
  return (
    <form
      className="grid gap-4"
      onSubmit={form.handleSubmit((values) => onReview(values))}
    >
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Choose exact fields that become public. Publication is separate from
        Commons contribution and profile attribution.
      </p>
      <label className="grid grid-cols-[1fr_auto] gap-2 text-[11px] font-bold text-muted-foreground">
        Page title
        <span className="font-mono text-[10px] font-normal text-muted-foreground">
          {title.length}/100
        </span>
        <Input
          {...form.register("title")}
          maxLength={100}
          disabled={working}
          aria-invalid={Boolean(errors.title)}
          aria-describedby={errors.title ? "public-title-error" : undefined}
        />
        <FormFieldError
          id="public-title-error"
          message={errors.title?.message}
        />
      </label>
      <label className="grid grid-cols-[1fr_auto] gap-2 text-[11px] font-bold text-muted-foreground">
        Public outcome summary
        <span className="font-mono text-[10px] font-normal text-muted-foreground">
          {outcomeSummary.length}/600
        </span>
        <Textarea
          {...form.register("outcome_summary")}
          rows={4}
          maxLength={600}
          disabled={working}
          aria-invalid={Boolean(errors.outcome_summary)}
          aria-describedby={
            errors.outcome_summary ? "public-summary-error" : undefined
          }
        />
        <FormFieldError
          id="public-summary-error"
          message={errors.outcome_summary?.message}
        />
      </label>
      <label className="grid grid-cols-[1fr_auto] gap-2 text-[11px] font-bold text-muted-foreground">
        Reusable instructions
        <span className="font-mono text-[10px] font-normal text-muted-foreground">
          {workflow.length}/4000
        </span>
        <Textarea
          {...form.register("workflow")}
          rows={6}
          maxLength={4000}
          disabled={working}
          aria-invalid={Boolean(errors.workflow)}
          aria-describedby={
            errors.workflow ? "public-workflow-error" : undefined
          }
        />
        <FormFieldError
          id="public-workflow-error"
          message={errors.workflow?.message}
        />
      </label>
      {correctionAvailable && (
        <label className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground">
          <Checkbox
            checked={correction.field.value !== null}
            onCheckedChange={(checked) =>
              correction.field.onChange(
                checked === true ? detail.human_correction : null,
              )
            }
            disabled={working}
            aria-invalid={Boolean(errors.correction_excerpt)}
            aria-describedby={
              errors.correction_excerpt ? "public-correction-error" : undefined
            }
          />
          <span>
            <strong>Publish contributed correction</strong>
            <small>Exact correction text will be shown publicly.</small>
          </span>
        </label>
      )}
      <FormFieldError
        id="public-correction-error"
        message={errors.correction_excerpt?.message}
      />
      <fieldset className="grid gap-px border-0 p-0">
        <legend>Supporting evidence</legend>
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          Select one to four exact excerpts from redacted contribution.
        </p>
        {detail.evidence.map((item) => {
          const checked = evidence.field.value.some(
            (selected) => selected.event_id === item.event_id,
          );
          return (
            <label
              className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
              key={item.event_id}
            >
              <Checkbox
                checked={checked}
                onCheckedChange={() => toggleEvidence(item.event_id, item.excerpt)}
                disabled={
                  working || (!checked && evidence.field.value.length >= 4)
                }
                aria-invalid={Boolean(errors.evidence)}
                aria-describedby={
                  errors.evidence ? "public-evidence-error" : undefined
                }
              />
              <span>
                <strong>{item.kind.replaceAll("_", " ")}</strong>
                <small>{item.excerpt}</small>
              </span>
            </label>
          );
        })}
        <FormFieldError
          id="public-evidence-error"
          message={errors.evidence?.message}
        />
      </fieldset>
      <fieldset className="grid gap-px border-0 p-0">
        <legend>Reuse permission</legend>
        <RadioGroup
          className="gap-px"
          value={reusePermission.field.value}
          onValueChange={(value) => reusePermission.field.onChange(value)}
        >
        {permissions.map((permission) => (
          <label
            className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
            key={permission.value}
          >
            <RadioGroupItem
              value={permission.value}
              disabled={working}
              aria-invalid={Boolean(errors.reuse_permission)}
              aria-describedby={
                errors.reuse_permission ? "public-permission-error" : undefined
              }
            />
            <span>
              <strong>{permission.label}</strong>
              <small>{permission.detail}</small>
            </span>
          </label>
        ))}
        </RadioGroup>
        <FormFieldError
          id="public-permission-error"
          message={errors.reuse_permission?.message}
        />
      </fieldset>
      <label className="grid grid-cols-[1fr_auto] gap-2 text-[11px] font-bold text-muted-foreground">
        Source public run
        <Input
          {...form.register("source")}
          placeholder="Optional tracecommons.ai/runs link or slug"
          disabled={working}
          aria-invalid={Boolean(errors.source)}
          aria-describedby={errors.source ? "public-source-error" : undefined}
        />
        <small>Add when workflow varies an existing public run.</small>
        <FormFieldError
          id="public-source-error"
          message={errors.source?.message}
        />
      </label>
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive m-0">
          {error}
        </p>
      )}
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onCancel}
          disabled={working}
        >
          Cancel
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="submit"
          disabled={working || !form.formState.isValid}
        >
          {working
            ? "Reviewing…"
            : editingPublished
              ? "Review update"
              : "Create public page"}
        </Button>
      </div>
    </form>
  );
}
