import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { ComparisonTaskDetail } from "../comparisons";
import {
  type ComparisonSpecificationFormValues,
  comparisonSpecificationFormSchema,
} from "../forms";
import type { ComparisonSpecificationInput } from "../specifications";

type SpecificationApi = {
  specifications: Array<
    ComparisonSpecificationInput & {
      id: string;
      specification_digest: string;
      estimator_state: { status: string };
      created_at: string;
    }
  >;
  preview: {
    specification: SpecificationApi["specifications"][number];
    result: {
      included_task_ids: string[];
      excluded_tasks: Array<{ task_id: string; reasons: string[] }>;
      cohorts: Array<{
        cohort_label: string;
        included_tasks: number;
        outcomes: Record<string, number>;
      }>;
    };
  } | null;
  result: {
    included_task_ids: string[];
    excluded_tasks: Array<{ task_id: string; reasons: string[] }>;
    cohorts: Array<{
      cohort_label: string;
      included_tasks: number;
      outcomes: Record<string, number>;
    }>;
    audit_digest: string;
  } | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  calculate: (input: ComparisonSpecificationInput) => void;
  save: (input: ComparisonSpecificationInput) => void;
  evaluate: (id: string) => void;
};

export function ComparisonSpecificationsPanel({
  tasks,
  specifications,
}: {
  tasks: ComparisonTaskDetail[];
  specifications: SpecificationApi;
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
      cutoff: new Date().toISOString().slice(0, 16),
    },
    mode: "onChange",
  });
  const busy =
    specifications.state === "busy" || specifications.state === "loading";
  useEffect(() => {
    if (form.formState.isDirty) return;
    const candidate = tasks.find(
      (task) => task.task.context && task.source_qualification,
    );
    if (!candidate?.task.context) return;
    const cohorts = tasks
      .flatMap((task) => task.source_qualification?.declared_model_cohort ?? [])
      .filter((value, index, all) => all.indexOf(value) === index)
      .join(", ");
    form.reset({
      projectId: candidate.task.context.project_id,
      language: candidate.task.context.language.value ?? "",
      fingerprint: candidate.task.context.configuration_fingerprint,
      cohorts,
      dateStart: candidate.task.context.task_date,
      dateEnd: candidate.task.context.task_date,
      cutoff: new Date().toISOString().slice(0, 16),
    });
  }, [form, tasks]);
  const input = (
    values: ComparisonSpecificationFormValues,
  ): ComparisonSpecificationInput => ({
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
  });
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            COMPARISON SPECIFICATIONS
          </span>
          <h2>Evaluate a fixed retrospective cohort</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          {specifications.specifications.length}
        </span>
      </div>
      <p>
        Specifications freeze date, cohort, and exact configuration inputs.
        Results remain descriptive; declared model cohorts are not verified
        identities.
      </p>
      {specifications.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {specifications.error}
        </p>
      )}
      <div className="mt-[22px] grid grid-cols-2 gap-4">
        <label>
          Project UUID
          <Input
            {...form.register("projectId")}
            aria-invalid={Boolean(form.formState.errors.projectId)}
            aria-describedby={
              form.formState.errors.projectId ? "spec-project-error" : undefined
            }
          />
          <FormFieldError
            id="spec-project-error"
            message={form.formState.errors.projectId?.message}
          />
        </label>
        <label>
          Language
          <Input
            {...form.register("language")}
            aria-invalid={Boolean(form.formState.errors.language)}
            aria-describedby={
              form.formState.errors.language ? "spec-language-error" : undefined
            }
          />
          <FormFieldError
            id="spec-language-error"
            message={form.formState.errors.language?.message}
          />
        </label>
        <label>
          Configuration fingerprint
          <Input
            {...form.register("fingerprint")}
            aria-invalid={Boolean(form.formState.errors.fingerprint)}
            aria-describedby={
              form.formState.errors.fingerprint
                ? "spec-fingerprint-error"
                : undefined
            }
          />
          <FormFieldError
            id="spec-fingerprint-error"
            message={form.formState.errors.fingerprint?.message}
          />
        </label>
        <label>
          Cohorts
          <Input
            {...form.register("cohorts")}
            placeholder="model-a, model-b"
            aria-invalid={Boolean(form.formState.errors.cohorts)}
            aria-describedby={
              form.formState.errors.cohorts ? "spec-cohorts-error" : undefined
            }
          />
          <FormFieldError
            id="spec-cohorts-error"
            message={form.formState.errors.cohorts?.message}
          />
        </label>
        <label>
          Start date
          <Input
            {...form.register("dateStart")}
            type="date"
            aria-invalid={Boolean(form.formState.errors.dateStart)}
            aria-describedby={
              form.formState.errors.dateStart ? "spec-start-error" : undefined
            }
          />
          <FormFieldError
            id="spec-start-error"
            message={form.formState.errors.dateStart?.message}
          />
        </label>
        <label>
          End date
          <Input
            {...form.register("dateEnd")}
            type="date"
            aria-invalid={Boolean(form.formState.errors.dateEnd)}
            aria-describedby={
              form.formState.errors.dateEnd ? "spec-end-error" : undefined
            }
          />
          <FormFieldError
            id="spec-end-error"
            message={form.formState.errors.dateEnd?.message}
          />
        </label>
        <label>
          Evidence cutoff
          <Input
            {...form.register("cutoff")}
            type="datetime-local"
            aria-invalid={Boolean(form.formState.errors.cutoff)}
            aria-describedby={
              form.formState.errors.cutoff ? "spec-cutoff-error" : undefined
            }
          />
          <FormFieldError
            id="spec-cutoff-error"
            message={form.formState.errors.cutoff?.message}
          />
        </label>
      </div>
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={() =>
            void form.handleSubmit((values) =>
              specifications.calculate(input(values)),
            )()
          }
          disabled={busy || !form.formState.isValid}
        >
          Preview specification
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={() =>
            void form.handleSubmit((values) =>
              specifications.save(input(values)),
            )()
          }
          disabled={busy || !form.formState.isValid}
        >
          Save specification
        </Button>
      </div>
      {specifications.preview && (
        <ResultView
          title="Preview result"
          result={specifications.preview.result}
        />
      )}
      <div className="mt-[22px] grid gap-px border-t border-border">
        {specifications.specifications.map((specification) => (
          <div
            className="grid grid-cols-[38px_minmax(0,1fr)_auto_auto] items-center gap-3.5 border-b border-border py-3.5 max-[860px]:grid-cols-[38px_minmax(0,1fr)_auto]"
            key={specification.id}
          >
            <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground bg-blue">
              S
            </span>
            <span className="grid min-w-0 gap-1">
              <strong>
                {specification.date_start} – {specification.date_end}
              </strong>
              <span>
                {specification.cohort_labels.join(" / ")} · {specification.id}
              </span>
            </span>
            <span className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
              <strong>{specification.estimator_state.status}</strong>
              <Button
                className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
                type="button"
                onClick={() => specifications.evaluate(specification.id)}
                disabled={busy}
              >
                Evaluate
              </Button>
            </span>
          </div>
        ))}
      </div>
      {specifications.result && (
        <ResultView title="Saved result" result={specifications.result} />
      )}
    </section>
  );
}

type ResultViewProps = {
  title: string;
  result: {
    included_task_ids: string[];
    excluded_tasks: Array<{ task_id: string; reasons: string[] }>;
    cohorts: Array<{
      cohort_label: string;
      included_tasks: number;
      outcomes: Record<string, number>;
    }>;
  };
};

function ResultView({ title, result }: ResultViewProps) {
  return (
    <div className="mt-[22px] border-t border-border pt-5">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        {title}
      </span>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Included tasks: {result.included_task_ids.length}. Excluded tasks:{" "}
        {result.excluded_tasks.length}. Accepted, partial, and rejected counts
        are conditional on eligible assessed evidence.
      </p>
      <div className="mt-[18px] grid grid-cols-2 gap-2.5 max-[860px]:grid-cols-1">
        {result.cohorts.map((cohort) => (
          <article
            className="grid gap-[6px] rounded-[10px] border border-border bg-muted p-3.5"
            key={cohort.cohort_label}
          >
            <strong>{cohort.cohort_label}</strong>
            <small>{cohort.included_tasks} included tasks</small>
            {Object.entries(cohort.outcomes).map(([key, value]) => (
              <div
                className="flex justify-between gap-3 border-t border-border pt-2 text-[11px] text-muted-foreground"
                key={key}
              >
                <span>{key}</span>
                <b>{value}</b>
              </div>
            ))}
          </article>
        ))}
      </div>
    </div>
  );
}
