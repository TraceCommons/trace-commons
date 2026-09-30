import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { Button } from "@/components/ui/button";
import type { ComparisonTaskDetail } from "../comparisons";
import {
  type ComparisonSpecificationFormValues,
  comparisonSpecificationFormSchema,
} from "../forms";
import type { ComparisonSpecificationInput } from "../specifications";
import { ComparisonSpecificationFields } from "./comparison-specification-fields";

function localDateTimeInputValue(date = new Date()): string {
  const localDate = new Date(
    date.getTime() - date.getTimezoneOffset() * 60_000,
  );
  return localDate.toISOString().slice(0, 16);
}

function inputFromValues(
  values: ComparisonSpecificationFormValues,
): ComparisonSpecificationInput {
  return {
    evidence_cutoff: new Date(values.cutoff).toISOString(),
    cohort_labels: values.cohorts
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean),
    date_start: values.dateStart,
    date_end: values.dateEnd,
    stratum: {
      project_id: values.projectId,
      language: values.language,
      configuration_fingerprint: values.fingerprint,
    },
  };
}

export function ComparisonSpecificationForm({
  tasks,
  busy,
  onCalculate,
  onSave,
}: {
  tasks: ComparisonTaskDetail[];
  busy: boolean;
  onCalculate: (input: ComparisonSpecificationInput) => Promise<void>;
  onSave: (input: ComparisonSpecificationInput) => Promise<void>;
}) {
  const form = useForm<ComparisonSpecificationFormValues>({
    resolver: zodResolver(comparisonSpecificationFormSchema),
    defaultValues: {
      projectId: "",
      language: "",
      fingerprint: "",
      cohorts: "",
      dateStart: "",
      dateEnd: "",
      cutoff: localDateTimeInputValue(),
    },
    mode: "onChange",
  });
  const { isDirty, isValid } = form.formState;
  const { reset } = form;

  useEffect(() => {
    if (isDirty) return;
    const candidate = tasks.find(
      (task) => task.task.context && task.source_qualification,
    );
    if (!candidate?.task.context) return;
    const cohortLabels = Array.from(
      new Set(
        tasks.flatMap(
          (task) => task.source_qualification?.declared_model_cohort ?? [],
        ),
      ),
    ).sort();
    // The schema requires exactly two sorted, unique labels. Leave incomplete
    // or multi-cohort data for the user to choose instead of seeding an invalid draft.
    const cohorts = cohortLabels.length === 2 ? cohortLabels.join(", ") : "";
    reset({
      projectId: candidate.task.context.project_id,
      language: candidate.task.context.language.value ?? "",
      fingerprint: candidate.task.context.configuration_fingerprint,
      cohorts,
      dateStart: candidate.task.context.task_date,
      dateEnd: candidate.task.context.task_date,
      cutoff: localDateTimeInputValue(),
    });
  }, [isDirty, reset, tasks]);

  const preview = form.handleSubmit((values) =>
    onCalculate(inputFromValues(values)),
  );
  const save = form.handleSubmit((values) => onSave(inputFromValues(values)));

  return (
    <>
      <ComparisonSpecificationFields form={form} />
      <div className="mt-3 flex flex-wrap gap-2">
        <Button
          className="tc-btn tc-btn--glass"
          type="button"
          onClick={() => void preview()}
          disabled={busy || !isValid}
        >
          Preview specification
        </Button>
        <Button
          className="tc-btn tc-btn--primary tc-btn--sm"
          type="button"
          onClick={() => void save()}
          disabled={busy || !isValid}
        >
          Save specification
        </Button>
      </div>
    </>
  );
}
