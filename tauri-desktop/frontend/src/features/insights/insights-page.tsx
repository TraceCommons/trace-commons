import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { Button } from "@/components/ui/button";
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
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="LOCAL / ANALYSIS"
        title="Insights"
        description="Choose a local rollout or trajectory and analyze it without uploading the source."
        phase="PHASE 2"
      />
      <div className="mb-4 grid grid-cols-3 gap-3 max-[860px]:grid-cols-1">
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
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {insights.error}
        </p>
      )}
      <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          ANALYSIS SOURCE
        </span>
        <h2>Analyze one local file</h2>
        <p>
          Selected bytes are passed to Rust for bounded local analysis. The
          original file is never modified or uploaded.
        </p>
        <div className="mt-6 flex gap-2.5">
          <label className="grid min-w-[175px] gap-1.5 text-[11px] font-bold text-muted-foreground">
            Format
            <NativeSelect
              {...sourceForm.register("source")}
              disabled={insights.state === "busy"}
              aria-invalid={Boolean(sourceForm.formState.errors.source)}
            >
              <option value="codex">Codex rollout</option>
              <option value="claude_code">Claude Code session</option>
              <option value="trajectory">Trajectory</option>
            </NativeSelect>
          </label>
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="button"
            onClick={() => input.current?.click()}
            disabled={insights.state === "busy"}
          >
            {insights.state === "busy" ? "Working…" : "Choose file"}
          </Button>
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={() => void insights.refresh()}
            disabled={insights.state === "busy"}
          >
            Refresh history
          </Button>
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
      <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
        <div className="flex items-start justify-between gap-[18px]">
          <div>
            <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
              SAVED SNAPSHOTS
            </span>
            <h2>Local history</h2>
          </div>
          <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
            {snapshots.length}
          </span>
        </div>
        {insights.state === "loading" ? (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            Reading local Insights store…
          </p>
        ) : snapshots.length === 0 ? (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            No saved insights. Choose a file to analyze; saving is optional.
          </p>
        ) : (
          <div className="mt-[22px] grid gap-px border-t border-border">
            {snapshots.map((snapshot) => (
              <Button
                className="grid w-full grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-0 border-b border-border bg-transparent py-3.5 text-left hover:bg-muted"
                type="button"
                key={snapshot.id}
                onClick={() => void insights.openSnapshot(snapshot.id)}
                disabled={insights.state === "busy"}
              >
                <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground bg-blue">
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
              </Button>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
