import { Button } from "@/components/ui/button";
import type { SkillCopy, SkillEvaluationReport } from "../skill-types";

export function SkillEvaluationResults({
  copy,
  report,
  busy,
  onReviewInstall,
  onInspect,
}: {
  copy: SkillCopy;
  report: SkillEvaluationReport;
  busy: boolean;
  onReviewInstall: () => void;
  onInspect: (url: string) => void;
}) {
  const candidate = report.summaries.find(
    (item) => item.arm === "candidate_skill",
  );
  return (
    <div className="grid gap-4">
      <div className="grid gap-[5px] border-l-[3px] border-chart-2 bg-background p-3.5">
        <strong>
          {report.install_allowed ? copy.passed_gate : copy.failed_gate}
        </strong>
        <span>{report.gate_reason}</span>
      </div>
      <div className="grid grid-cols-2 gap-3 max-[860px]:grid-cols-1">
        <ResultGroup
          title={copy.repository_plans}
          items={report.plan_summaries}
        />
        <ResultGroup
          title={copy.skill_applicability}
          items={report.applicability_summaries}
        />
      </div>
      <div className="my-5 flex flex-wrap gap-x-[26px] gap-y-2 text-[11px] text-muted-foreground">
        <span>
          <b>{copy.model_and_budget}</b>
          {report.served_model}
        </span>
        <span>
          <b>{copy.passed}</b>
          {candidate?.passed ?? 0}
        </span>
        <span>
          <b>{copy.failed}</b>
          {(candidate?.total ?? 0) - (candidate?.passed ?? 0)}
        </span>
      </div>
      {report.regressions.length > 0 ? (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {copy.regressions}: {report.regressions.join(", ")}
        </p>
      ) : (
        <p className="mt-[15px] text-[11px] leading-[1.5] text-primary">
          {copy.no_regressions}
        </p>
      )}
      <div className="grid gap-px border-t border-border pt-4">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          {copy.inspect_runs}
        </span>
        {report.trials.map((trial) => (
          <details key={`${trial.task_id}-${trial.arm}`}>
            <summary>
              <span>{trial.task_id}</span>
              <strong className={trial.passed ? "skill-pass" : "skill-fail"}>
                {trial.passed ? copy.passed : copy.failed}
              </strong>
              <small>{trial.arm}</small>
            </summary>
            <div className="grid gap-2 pb-[13px] text-[11px] leading-[1.5] text-muted-foreground">
              <p>{trial.task}</p>
              {trial.source_url && (
                <Button
                  className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
                  type="button"
                  onClick={() => onInspect(trial.source_url)}
                >
                  {copy.open_fixture_source}
                </Button>
              )}
              {trial.answer && (
                <>
                  <b>{copy.edits}</b>
                  <span>{trial.answer.edit_paths.join(" · ") || "—"}</span>
                  <b>{copy.commands}</b>
                  <span>{trial.answer.commands.join(" · ") || "—"}</span>
                  <b>{copy.checks}</b>
                  <span>{trial.answer.verification.join(" · ") || "—"}</span>
                </>
              )}
              {trial.failure_reasons.length > 0 && (
                <span className="text-destructive">
                  {trial.failure_reasons.join(" · ")}
                </span>
              )}
              <details>
                <summary>{copy.model_output}</summary>
                <pre>{trial.raw_output}</pre>
              </details>
            </div>
          </details>
        ))}
      </div>
      {report.install_allowed && (
        <div className="mt-6 flex gap-2.5">
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="button"
            onClick={onReviewInstall}
            disabled={busy}
          >
            {busy ? copy.preparing : copy.review_install}
          </Button>
        </div>
      )}
    </div>
  );
}

function ResultGroup({
  title,
  items,
}: {
  title: string;
  items: SkillEvaluationReport["summaries"];
}) {
  return (
    <div className="grid gap-px border-t border-border pt-3">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        {title}
      </span>
      {items.map((item) => (
        <div key={item.arm}>
          <span>{item.label}</span>
          <strong>
            {item.passed}/{item.total}
          </strong>
        </div>
      ))}
    </div>
  );
}
