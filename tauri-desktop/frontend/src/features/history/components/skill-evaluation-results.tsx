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
      <div className="grid gap-[5px] tc-card tc-card--quiet">
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
      <div className="my-2.5 flex flex-wrap gap-x-4 gap-y-1.5 tc-caption tc-text-tertiary">
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
        <p className="tc-alert">
          {copy.regressions}: {report.regressions.join(", ")}
        </p>
      ) : (
        <p className="mt-2 tc-caption tc-text-accent">
          {copy.no_regressions}
        </p>
      )}
      <div className="grid gap-px pt-2">
        <span className="mb-1.5 block tc-eyebrow">
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
                  className="tc-link"
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
        <div className="mt-3 flex flex-wrap gap-2">
          <Button
            className="tc-btn tc-btn--primary tc-btn--sm"
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
      <span className="mb-1.5 block tc-eyebrow">
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
