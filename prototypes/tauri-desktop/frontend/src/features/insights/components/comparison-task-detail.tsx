import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import type { UseFormReturn } from "react-hook-form";
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
        {episodes.map((entry) => (
          <label
            className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
            key={entry.episode.id}
          >
            <Checkbox
              checked={editSelection.value.includes(entry.episode.id)}
              onCheckedChange={() => onToggle(editSelection, entry.episode.id)}
              disabled={busy}
            />
            <span>
              <strong>{entry.episode.members.length} snapshots</strong>
              <small>{entry.episode.id}</small>
            </span>
          </label>
        ))}
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
        <label>
          Project UUID
          <Input
            {...form.register("projectId")}
            disabled={busy}
            aria-invalid={Boolean(form.formState.errors.projectId)}
            aria-describedby={
              form.formState.errors.projectId ? "task-project-error" : undefined
            }
          />
          <FormFieldError
            id="task-project-error"
            message={form.formState.errors.projectId?.message}
          />
        </label>
        <label>
          Task date
          <Input
            type="date"
            {...form.register("taskDate")}
            disabled={busy}
            aria-invalid={Boolean(form.formState.errors.taskDate)}
            aria-describedby={
              form.formState.errors.taskDate ? "task-date-error" : undefined
            }
          />
          <FormFieldError
            id="task-date-error"
            message={form.formState.errors.taskDate?.message}
          />
        </label>
        <label>
          Language
          <Input
            {...form.register("language")}
            placeholder="rust"
            disabled={busy}
          />
        </label>
        <label>
          Harness ID
          <Input {...form.register("harnessId")} disabled={busy} />
        </label>
        <label>
          Harness version
          <Input {...form.register("harnessVersion")} disabled={busy} />
        </label>
        <label>
          Tool policy ID
          <Input {...form.register("toolPolicyId")} disabled={busy} />
        </label>
        <label>
          Tool policy version
          <Input {...form.register("toolPolicyVersion")} disabled={busy} />
        </label>
        <label>
          Prompt template digest
          <Input
            {...form.register("promptDigest")}
            placeholder="64 lowercase hex characters"
            disabled={busy}
            aria-invalid={Boolean(form.formState.errors.promptDigest)}
            aria-describedby={
              form.formState.errors.promptDigest
                ? "task-prompt-digest-error"
                : undefined
            }
          />
          <FormFieldError
            id="task-prompt-digest-error"
            message={form.formState.errors.promptDigest?.message}
          />
        </label>
        <label>
          Reasoning effort
          <NativeSelect {...form.register("reasoning")} disabled={busy}>
            <option value="unknown">Unknown</option>
            <option value="none">None</option>
            <option value="minimal">Minimal</option>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
            <option value="xhigh">Xhigh</option>
          </NativeSelect>
        </label>
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
        <label>
          Outcome
          <NativeSelect {...form.register("outcome")} disabled={busy}>
            <option value="pending">Pending</option>
            <option value="accepted">Accepted</option>
            <option value="partial">Partial</option>
            <option value="rejected">Rejected</option>
            <option value="unknown">Unknown</option>
          </NativeSelect>
        </label>
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
        Checkout provenance remains unavailable in this local prototype.
        Reconfirmation binds current material digest; it does not verify model
        identity or task independence.
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
