import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useRef } from "react";
import { useController, useForm } from "react-hook-form";
import type { ComparisonTaskDetail as ComparisonTaskDetailData } from "../comparisons";
import {
  type ComparisonTaskFormValues,
  comparisonTaskFormSchema,
} from "../forms";
import type { EpisodeListEntry } from "../workflows";
import { ComparisonTaskDetail } from "./comparison-task-detail";
import { ComparisonTaskList } from "./comparison-task-list";
import type {
  ComparisonTasksApi,
  SelectionField,
} from "./comparison-task-types";

const today = new Date().toISOString().slice(0, 10);
const projectDefault = "00000000-0000-4000-8000-000000000001";

function knownConfigurationValue(
  configuration: Record<string, unknown>,
  key: string,
  property: "value" | "digest" = "value",
) {
  const entry = configuration[key];
  if (property === "value" && typeof entry === "string") return entry;
  if (typeof entry !== "object" || entry === null) return "";
  const value = (entry as Record<string, unknown>)[property];
  return typeof value === "string" ? value : "";
}

function formValuesForDetail(
  detail: ComparisonTaskDetailData | null,
): ComparisonTaskFormValues {
  const context = detail?.task.context;
  const configuration = context?.configuration ?? {};
  const reasoning = knownConfigurationValue(configuration, "reasoning_effort");
  const outcome = detail?.task.outcome?.value;
  return {
    episodeSelection: [],
    editSelection: [],
    projectId: context?.project_id ?? projectDefault,
    taskDate: context?.task_date ?? today,
    language: context?.language.value ?? "",
    harnessId: knownConfigurationValue(configuration, "harness_id"),
    harnessVersion: knownConfigurationValue(configuration, "harness_version"),
    toolPolicyId: knownConfigurationValue(configuration, "tool_policy_id"),
    toolPolicyVersion: knownConfigurationValue(
      configuration,
      "tool_policy_version",
    ),
    promptDigest: knownConfigurationValue(
      configuration,
      "prompt_template_digest",
      "digest",
    ),
    reasoning: [
      "unknown",
      "none",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
    ].includes(reasoning)
      ? (reasoning as ComparisonTaskFormValues["reasoning"])
      : "unknown",
    outcome: ["pending", "accepted", "partial", "rejected", "unknown"].includes(
      outcome ?? "",
    )
      ? (outcome as ComparisonTaskFormValues["outcome"])
      : "unknown",
  };
}

function toggleSelection(field: SelectionField, id: string) {
  field.onChange(
    field.value.includes(id)
      ? field.value.filter((value) => value !== id)
      : [...field.value, id],
  );
}

export function ComparisonTasksPanel({
  episodes,
  comparison,
}: {
  episodes: EpisodeListEntry[];
  comparison: ComparisonTasksApi;
}) {
  const form = useForm<ComparisonTaskFormValues>({
    resolver: zodResolver(comparisonTaskFormSchema),
    defaultValues: formValuesForDetail(null),
    mode: "onChange",
  });
  const detail = comparison.detail;
  const previousDetailId = useRef<string | null>(null);
  useEffect(() => {
    const detailId = detail?.task.id ?? null;
    if (detailId === previousDetailId.current) return;
    previousDetailId.current = detailId;
    form.reset(formValuesForDetail(detail));
  }, [detail, form]);
  const episodeSelection = useController({
    control: form.control,
    name: "episodeSelection",
  });
  const editSelection = useController({
    control: form.control,
    name: "editSelection",
  });
  const busy = comparison.state === "busy" || comparison.state === "loading";

  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            COMPARISON TASKS
          </span>
          <h2>Freeze reviewed episode evidence</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          {comparison.tasks.length}
        </span>
      </div>
      <p>
        Tasks freeze selected episode membership for retrospective review. A
        task is not a verified outcome until context, outcome, attribution, and
        independence evidence are separately reviewed.
      </p>
      {comparison.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {comparison.error}
        </p>
      )}
      {detail ? (
        <ComparisonTaskDetail
          detail={detail}
          episodes={episodes}
          form={form}
          editSelection={editSelection.field}
          busy={busy}
          onClose={comparison.close}
          onToggle={toggleSelection}
          onReplace={comparison.replaceEpisodes}
          onSetContext={comparison.setContext}
          onSetOutcome={comparison.setOutcome}
          onClearOutcome={comparison.clearOutcome}
          onReconfirm={comparison.reconfirm}
        />
      ) : (
        <ComparisonTaskList
          episodes={episodes}
          tasks={comparison.tasks}
          form={form}
          episodeSelection={episodeSelection.field}
          busy={busy}
          onToggle={toggleSelection}
          onCreate={comparison.create}
          onOpen={comparison.open}
        />
      )}
    </section>
  );
}
