import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useRef } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { type AnnotationFormValues, annotationFormSchema } from "../forms";
import type { Insight } from "../types";
import { InsightEvidencePanel } from "./insight-evidence-panel";

type InsightDetailProps = {
  insight: Insight;
  saved: boolean;
  busy: boolean;
  onSave: () => void;
  onDelete: () => void;
  onAnnotate: (category: string, outcome: string) => Promise<boolean>;
  onClearAnnotation: () => Promise<boolean>;
  onLinkTestReport: (file: File) => void;
  onLinkGit: (repository: string, commit: string) => Promise<boolean>;
  onUnlinkEvidence: (id: string) => void;
};

export function InsightDetail({
  insight,
  saved,
  busy,
  onSave,
  onDelete,
  onAnnotate,
  onClearAnnotation,
  onLinkTestReport,
  onLinkGit,
  onUnlinkEvidence,
}: InsightDetailProps) {
  const form = useForm<AnnotationFormValues>({
    resolver: zodResolver(annotationFormSchema),
    defaultValues: {
      category: (insight.manual_annotation?.category ??
        "unknown") as AnnotationFormValues["category"],
      outcome: (insight.manual_annotation?.outcome ??
        "unknown") as AnnotationFormValues["outcome"],
    },
  });
  const previousInsightId = useRef<string | null>(null);
  useEffect(() => {
    const identityChanged = previousInsightId.current !== insight.id;
    previousInsightId.current = insight.id;
    if (identityChanged || !form.formState.isDirty) {
      form.reset({
        category: (insight.manual_annotation?.category ??
          "unknown") as AnnotationFormValues["category"],
        outcome: (insight.manual_annotation?.outcome ??
          "unknown") as AnnotationFormValues["outcome"],
      });
    }
  }, [form, insight]);
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            {saved ? "SAVED SNAPSHOT" : "ANALYSIS RESULT"}
          </span>
          <h2>{formatSource(insight.source_format)}</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          {formatDate(insight.analyzed_at)}
        </span>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        {insight.boundary}
      </p>
      <div className="my-5 flex flex-wrap gap-x-[26px] gap-y-2 text-[11px] text-muted-foreground">
        <span>
          <b>Analyzer</b>
          {insight.report.provider.id} {insight.report.provider.version}
        </span>
        <span>
          <b>Rubric</b>
          {insight.report.provider.rubric_version}
        </span>
        <span>
          <b>Mode</b>
          {insight.report.provider.execution_mode}
        </span>
      </div>
      <div className="grid grid-cols-3 gap-[9px]">
        {insight.report.metrics.map((metric) => (
          <div
            className="grid gap-[6px] rounded-[10px] border border-border bg-muted p-3.5"
            key={metric.id}
          >
            <span>{metric.id}</span>
            <strong>{metric.value === null ? "Unknown" : metric.value}</strong>
            <small>
              {metric.coverage.observed}/{metric.coverage.total} recognized
            </small>
          </div>
        ))}
      </div>
      <form
        className="mt-6 border-t border-border pt-5"
        onSubmit={form.handleSubmit(async (values) => {
          if (await onAnnotate(values.category, values.outcome))
            form.reset(values);
        })}
      >
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          USER-REPORTED ASSESSMENT
        </span>
        <div className="my-3 flex gap-3">
          <label>
            Category
            <NativeSelect
              {...form.register("category")}
              disabled={!saved || busy}
              aria-invalid={Boolean(form.formState.errors.category)}
              aria-describedby={
                form.formState.errors.category
                  ? "insight-category-error"
                  : undefined
              }
            >
              <option value="unknown">Unknown</option>
              <option value="refactor">Refactor</option>
              <option value="tests">Tests</option>
              <option value="docs">Documentation</option>
              <option value="debugging">Debugging</option>
              <option value="other">Other</option>
            </NativeSelect>
          </label>
          <label>
            Outcome
            <NativeSelect
              {...form.register("outcome")}
              disabled={!saved || busy}
              aria-invalid={Boolean(form.formState.errors.outcome)}
              aria-describedby={
                form.formState.errors.outcome
                  ? "insight-outcome-error"
                  : undefined
              }
            >
              <option value="unknown">Unknown</option>
              <option value="accepted">Accepted</option>
              <option value="partial">Partial</option>
              <option value="rejected">Rejected</option>
            </NativeSelect>
          </label>
        </div>
        <FormFieldError
          id="insight-category-error"
          message={form.formState.errors.category?.message}
        />
        <FormFieldError
          id="insight-outcome-error"
          message={form.formState.errors.outcome?.message}
        />
        <div className="flex flex-wrap justify-end gap-[9px]">
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={async () => {
              if (await onClearAnnotation())
                form.reset({ category: "unknown", outcome: "unknown" });
            }}
            disabled={!saved || busy}
          >
            Clear assessment
          </Button>
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="submit"
            disabled={!saved || busy}
          >
            Save assessment
          </Button>
        </div>
      </form>
      <InsightEvidencePanel
        insight={insight}
        saved={saved}
        busy={busy}
        onLinkTestReport={onLinkTestReport}
        onLinkGit={onLinkGit}
        onUnlink={onUnlinkEvidence}
      />
      <div className="mt-[18px] flex justify-end gap-[9px]">
        {saved ? (
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
            type="button"
            onClick={onDelete}
            disabled={busy}
          >
            Delete saved insight
          </Button>
        ) : (
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="button"
            onClick={onSave}
            disabled={busy}
          >
            Re-read and save
          </Button>
        )}
      </div>
    </section>
  );
}

function formatSource(source: string) {
  return source === "claude_code"
    ? "Claude Code session"
    : source === "trajectory"
      ? "Trajectory"
      : "Codex rollout";
}
function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString();
}
