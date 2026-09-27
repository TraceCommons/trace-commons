import { useId } from "react";
import type { UseFormReturn } from "react-hook-form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { FormFieldError } from "../../../components/form-field-error";
import type {
  ComparisonContext,
  ComparisonTaskDetail as ComparisonTaskDetailData,
} from "../comparisons";
import type { ComparisonTaskFormValues } from "../forms";
import type { EpisodeListEntry } from "../workflows";
import type { SelectionField } from "./comparison-task-types";

export function ComparisonTaskDetail({
  detail,
  episodes,
  form,
  editSelection,
  busy,
  onClose,
  onToggle,
  onReplace,
  onSetContext,
  onSetOutcome,
  onClearOutcome,
  onReconfirm,
}: {
  detail: ComparisonTaskDetailData;
  episodes: EpisodeListEntry[];
  form: UseFormReturn<ComparisonTaskFormValues>;
  editSelection: SelectionField;
  busy: boolean;
  onClose: () => void;
  onToggle: (field: SelectionField, id: string) => void;
  onReplace: (ids: string[]) => Promise<boolean>;
  onSetContext: (context: ComparisonContext) => Promise<boolean>;
  onSetOutcome: (value: string) => Promise<boolean>;
  onClearOutcome: () => Promise<boolean>;
  onReconfirm: () => Promise<boolean>;
}) {
  const idPrefix = useId();

  return (
    <div className="mt-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            TASK DETAIL
          </span>
          <h3>{detail.task.id}</h3>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={onClose}
        >
          Back to tasks
        </Button>
      </div>
      <div className="my-5 flex flex-wrap gap-x-[26px] gap-y-2 text-[11px] text-muted-foreground">
        <span>
          <b>Material</b>
          {detail.task.material_digest}
        </span>
        <span>
          <b>Revision</b>
          {detail.task.revision}
        </span>
        <span>
          <b>Resolved</b>
          {new Date(detail.resolved_at).toLocaleString()}
        </span>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        {detail.stale_reasons.length
          ? `Review required: ${detail.stale_reasons.join(", ")}`
          : "No stale reason reported."}{" "}
        {detail.overlapping_task_ids.length
          ? `Overlaps ${detail.overlapping_task_ids.length} other tasks.`
          : ""}
      </p>
      <div className="my-[18px] grid gap-px border-t border-border">
        {episodes.map((entry, index) => {
          const episodeId = `${idPrefix}-episode-${index}`;
          return (
            <div
              className="flex items-start gap-2.5 border-b border-border py-2.5"
              key={entry.episode.id}
            >
              <Checkbox
                id={episodeId}
                checked={editSelection.value.includes(entry.episode.id)}
                onCheckedChange={() =>
                  onToggle(editSelection, entry.episode.id)
                }
                disabled={busy}
              />
              <Label
                className="text-[12px] font-normal text-foreground"
                htmlFor={episodeId}
              >
                <span>
                  <strong>{entry.episode.members.length} snapshots</strong>
                  <small>{entry.episode.id}</small>
                </span>
              </Label>
            </div>
          );
        })}
      </div>
      <Button
        className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
        type="button"
        onClick={() =>
          void form.handleSubmit(async (values) => {
            if (await onReplace(values.editSelection))
              form.setValue("editSelection", [], { shouldDirty: false });
          })()
        }
        disabled={busy || editSelection.value.length === 0}
      >
        Replace frozen episodes
      </Button>
      <div className="mt-[22px] grid grid-cols-2 gap-4">
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-project-id`}>Project UUID</Label>
          <Input
            id={`${idPrefix}-project-id`}
            {...form.register("projectId")}
            disabled={busy}
            aria-invalid={Boolean(form.formState.errors.projectId)}
            aria-describedby={
              form.formState.errors.projectId
                ? `${idPrefix}-task-project-error`
                : undefined
            }
          />
          <FormFieldError
            id={`${idPrefix}-task-project-error`}
            message={form.formState.errors.projectId?.message}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-task-date`}>Task date</Label>
          <Input
            id={`${idPrefix}-task-date`}
            type="date"
            {...form.register("taskDate")}
            disabled={busy}
            aria-invalid={Boolean(form.formState.errors.taskDate)}
            aria-describedby={
              form.formState.errors.taskDate
                ? `${idPrefix}-task-date-error`
                : undefined
            }
          />
          <FormFieldError
            id={`${idPrefix}-task-date-error`}
            message={form.formState.errors.taskDate?.message}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-language`}>Language</Label>
          <Input
            id={`${idPrefix}-language`}
            {...form.register("language")}
            placeholder="rust"
            disabled={busy}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-harness-id`}>Harness ID</Label>
          <Input
            id={`${idPrefix}-harness-id`}
            {...form.register("harnessId")}
            disabled={busy}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-harness-version`}>Harness version</Label>
          <Input
            id={`${idPrefix}-harness-version`}
            {...form.register("harnessVersion")}
            disabled={busy}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-tool-policy-id`}>Tool policy ID</Label>
          <Input
            id={`${idPrefix}-tool-policy-id`}
            {...form.register("toolPolicyId")}
            disabled={busy}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-tool-policy-version`}>
            Tool policy version
          </Label>
          <Input
            id={`${idPrefix}-tool-policy-version`}
            {...form.register("toolPolicyVersion")}
            disabled={busy}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-prompt-digest`}>
            Prompt template digest
          </Label>
          <Input
            id={`${idPrefix}-prompt-digest`}
            {...form.register("promptDigest")}
            placeholder="64 lowercase hex characters"
            disabled={busy}
            aria-invalid={Boolean(form.formState.errors.promptDigest)}
            aria-describedby={
              form.formState.errors.promptDigest
                ? `${idPrefix}-task-prompt-digest-error`
                : undefined
            }
          />
          <FormFieldError
            id={`${idPrefix}-task-prompt-digest-error`}
            message={form.formState.errors.promptDigest?.message}
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-reasoning`}>Reasoning effort</Label>
          <NativeSelect
            id={`${idPrefix}-reasoning`}
            {...form.register("reasoning")}
            disabled={busy}
          >
            <option value="unknown">Unknown</option>
            <option value="none">None</option>
            <option value="minimal">Minimal</option>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
            <option value="xhigh">Xhigh</option>
          </NativeSelect>
        </div>
      </div>
      <Button
        className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
        type="button"
        onClick={() =>
          void form.handleSubmit(async (values) => {
            if (await onSetContext(toContext(values))) form.reset(values);
          })()
        }
        disabled={busy || !form.formState.isValid}
      >
        Save task context
      </Button>
      <FormFieldError
        id="task-form-error"
        message={form.formState.errors.root?.message}
      />
      <div className="my-3 flex gap-3">
        <div className="grid gap-1">
          <Label htmlFor={`${idPrefix}-outcome`}>Outcome</Label>
          <NativeSelect
            id={`${idPrefix}-outcome`}
            {...form.register("outcome")}
            disabled={busy}
          >
            <option value="pending">Pending</option>
            <option value="accepted">Accepted</option>
            <option value="partial">Partial</option>
            <option value="rejected">Rejected</option>
            <option value="unknown">Unknown</option>
          </NativeSelect>
        </div>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={() =>
            void form.handleSubmit(async (values) => {
              if (await onSetOutcome(values.outcome)) form.reset(values);
            })()
          }
          disabled={busy || !form.formState.isValid}
        >
          Save outcome
        </Button>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={async () => {
            if (await onClearOutcome())
              form.reset({ ...form.getValues(), outcome: "unknown" });
          }}
          disabled={busy || !detail.task.outcome}
        >
          Clear outcome
        </Button>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={() => void onReconfirm()}
          disabled={busy}
        >
          Reconfirm current material
        </Button>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Checkout provenance is unavailable in this desktop app. Reconfirmation
        binds current material digest; it does not verify model identity or task
        independence.
      </p>
    </div>
  );
}

function toContext(values: ComparisonTaskFormValues): ComparisonContext {
  return {
    project_id: values.projectId,
    category: "refactor",
    task_date: values.taskDate,
    checkout_provenance: { state: "unavailable" },
    language: values.language
      ? { state: "known", value: values.language }
      : { state: "unknown" },
    configuration: {
      harness_id: values.harnessId
        ? { state: "known", value: values.harnessId }
        : { state: "unknown" },
      harness_version: values.harnessVersion
        ? { state: "known", value: values.harnessVersion }
        : { state: "unknown" },
      reasoning_effort: values.reasoning,
      tool_policy_id: values.toolPolicyId
        ? { state: "known", value: values.toolPolicyId }
        : { state: "unknown" },
      tool_policy_version: values.toolPolicyVersion
        ? { state: "known", value: values.toolPolicyVersion }
        : { state: "unknown" },
      prompt_template_digest: values.promptDigest
        ? { state: "known", digest: values.promptDigest }
        : { state: "unknown" },
    },
    configuration_fingerprint: "",
  };
}
