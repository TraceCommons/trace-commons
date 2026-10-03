import { zodResolver } from "@hookform/resolvers/zod";
import { useRef } from "react";
import { useForm } from "react-hook-form";
import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import { ComparisonSpecificationsPanel } from "./components/comparison-specifications-panel";
import { ComparisonTasksPanel } from "./components/comparison-tasks-panel";
import { InsightDetail } from "./components/insight-detail";
import { InsightSummary } from "./components/insight-summary";
import { InsightsWorkflowsPanel } from "./components/insights-workflows-panel";
import { type AnalysisSourceValues, analysisSourceSchema } from "./forms";
import { useComparisonSpecifications } from "./hooks/use-comparison-specifications";
import { useComparisonTasks } from "./hooks/use-comparison-tasks";
import { useInsightsData } from "./hooks/use-insights-data";
import { useInsightsWorkflows } from "./hooks/use-insights-workflows";
import { ButtonPrimary, GlassButton, Input, Select } from "@/design-system";

export function InsightsPage() {
  const insights = useInsightsData();
  const workflow = useInsightsWorkflows();
  const comparison = useComparisonTasks();
  const specifications = useComparisonSpecifications();
  const input = useRef<HTMLInputElement>(null);
  const sourceForm = useForm<AnalysisSourceValues>({
    resolver: zodResolver(analysisSourceSchema),
    defaultValues: { source: "codex", file: undefined },
  });
  const source = sourceForm.watch("source");
  const fileField = sourceForm.register("file");
  const snapshots = insights.data?.snapshots ?? [];
  return (
    <div className="tc-page">
      <PageHeader
        eyebrow="LOCAL / ANALYSIS"
        title="Insights"
        description="Choose a local rollout or trajectory and analyze it without uploading the source."
        phase="PHASE 2"
      />
      <div className="mb-2.5 grid grid-cols-3 gap-1.5">
        <StatCard
          label="Storage"
          value="Local"
          detail="Saved snapshots stay on this device"
          tone="blue"
        />
        <StatCard
          label="Snapshots"
          value={insights.data ? `${snapshots.length}` : "—"}
          detail="Derived observations"
        />
        <StatCard
          label="Evidence"
          value="Explicit"
          detail="Assessments remain user-reported"
          tone="gold"
        />
      </div>
      {insights.state === "error" && (
        <p className="tc-alert">
          {insights.error}
        </p>
      )}
      <section className="tc-card mb-2.5">
        <span className="mb-1.5 block tc-eyebrow">
          ANALYSIS SOURCE
        </span>
        <h2>Analyze one local file</h2>
        <p>
          Selected bytes are passed to Rust for bounded local analysis. The
          original file is never modified or uploaded.
        </p>
        <div className="mt-3 flex flex-wrap gap-2">
          <label className="grid min-w-[175px] gap-1.5 text-[11px] font-bold text-tc-secondary">
            Format
            <Select
              {...sourceForm.register("source")}
              disabled={insights.state === "busy"}
              aria-invalid={Boolean(sourceForm.formState.errors.source)}
            >
              <option value="codex">Codex rollout</option>
              <option value="claude_code">Claude Code session</option>
              <option value="trajectory">Trajectory</option>
            </Select>
          </label>
          <ButtonPrimary size="sm"
            type="button"
            onClick={() => input.current?.click()}
            disabled={insights.state === "busy"}
          >
            {insights.state === "busy" ? "Working…" : "Choose file"}
          </ButtonPrimary>
          <GlassButton
            type="button"
            onClick={() => void insights.refresh()}
            disabled={insights.state === "busy"}
          >
            Refresh history
          </GlassButton>
        </div>
        <Input
          {...fileField}
          ref={(element) => {
            fileField.ref(element);
            input.current = element;
          }}
          className="absolute h-px w-px overflow-hidden whitespace-nowrap [clip:rect(0_0_0_0)] [clip-path:inset(50%)]"
          type="file"
          onChange={(event) => {
            const files = event.currentTarget.files;
            sourceForm.setValue("file", files, { shouldDirty: true });
            const file = files?.[0];
            sourceForm.resetField("file");
            event.currentTarget.value = "";
            if (file) void insights.analyze(file, source);
          }}
        />
      </section>
      {insights.data?.summary && (
        <InsightSummary summary={insights.data.summary} />
      )}
      {insights.selected && (
        <InsightDetail
          insight={insights.selected}
          saved={insights.selectedIsSaved}
          busy={insights.state === "busy"}
          onSave={() => void insights.save()}
          onDelete={() => void insights.remove()}
          onAnnotate={insights.annotate}
          onClearAnnotation={insights.clearAnnotation}
          onLinkTestReport={(file) => void insights.linkReport(file)}
          onLinkGit={insights.linkRepository}
          onUnlinkEvidence={(id) => void insights.unlink(id)}
        />
      )}
      <InsightsWorkflowsPanel snapshots={snapshots} workflow={workflow} />
      <ComparisonTasksPanel
        episodes={workflow.episodes}
        comparison={comparison}
      />
      <ComparisonSpecificationsPanel
        tasks={comparison.tasks}
        specifications={specifications}
      />
      <section className="tc-card">
        <div className="flex items-start justify-between gap-3">
          <div>
            <span className="mb-1.5 block tc-eyebrow">
              SAVED SNAPSHOTS
            </span>
            <h2>Local history</h2>
          </div>
          <span className="tc-chip tc-chip--glass self-start">
            {snapshots.length}
          </span>
        </div>
        {insights.state === "loading" ? (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
            Reading local Insights store…
          </p>
        ) : snapshots.length === 0 ? (
          <p className="mt-3 mb-1 tc-body tc-text-tertiary">
            No saved insights. Choose a file to analyze; saving is optional.
          </p>
        ) : (
          <div className="mt-3 grid gap-px">
            {snapshots.map((snapshot) => (
              <button
                className="grid w-full grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-lg border-0 bg-transparent px-1.5 py-2 text-left text-inherit hover:bg-white/5 tc-hairline-bottom"
                type="button"
                key={snapshot.id}
                onClick={() => void insights.openSnapshot(snapshot.id)}
                disabled={insights.state === "busy"}
              >
                <span className="tc-tool-tile tc-tool-tile--lg tc-tool-tile--folder">
                  {snapshot.source_format.slice(0, 1).toUpperCase()}
                </span>
                <span className="grid min-w-0 gap-1">
                  <strong>{snapshot.source_format}</strong>
                  <span>
                    {snapshot.report.provider.id} · {snapshot.id}
                  </span>
                </span>
                <span className="grid min-w-[116px] gap-1 text-right max-[860px]:hidden">
                  <strong>
                    {new Date(snapshot.analyzed_at).toLocaleDateString()}
                  </strong>
                  <span>
                    {snapshot.manual_annotation?.outcome ?? "Unassessed"}
                  </span>
                </span>
              </button>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
