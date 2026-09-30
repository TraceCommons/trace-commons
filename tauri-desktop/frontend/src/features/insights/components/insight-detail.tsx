import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useRef } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { ConfirmActionButton } from "../../../components/confirm-action-button";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
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
  const contributorCopy = useContributorDisclosureCopy();
  const deleteCopy = contributorCopy.data?.insights_ui;
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
    <section className="tc-card mb-2.5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            {saved ? "SAVED SNAPSHOT" : "ANALYSIS RESULT"}
          </span>
          <h2>{formatSource(insight.source_format)}</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
          {formatDate(insight.analyzed_at)}
        </span>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        {insight.boundary}
      </p>
      <div className="my-2.5 flex flex-wrap gap-x-4 gap-y-1.5 tc-caption tc-text-tertiary">
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
            className="tc-card tc-card--quiet grid gap-[6px]"
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
        className="mt-3 pt-3 tc-hairline-top"
        onSubmit={form.handleSubmit(async (values) => {
          if (await onAnnotate(values.category, values.outcome))
            form.reset(values);
        })}
      >
        <span className="mb-1.5 block tc-eyebrow">
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
            className="tc-btn tc-btn--glass"
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
            className="tc-btn tc-btn--glass"
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
      <div className="mt-3 flex justify-end gap-2">
        {saved ? (
          <ConfirmActionButton
            label={deleteCopy?.delete}
            title={deleteCopy?.delete}
            description={deleteCopy?.delete_confirm}
            confirmLabel={deleteCopy?.delete}
            cancelLabel={deleteCopy?.cancel}
            workingLabel={deleteCopy?.working}
            unavailableLabel="Delete confirmation copy unavailable. Reload before deleting."
            busy={busy}
            onConfirm={onDelete}
          />
        ) : (
          <Button
            className="tc-btn tc-btn--primary tc-btn--sm"
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
