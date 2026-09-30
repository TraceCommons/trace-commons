import type { ComparisonResult } from "../specifications";

export function ComparisonSpecificationResult({
  title,
  result,
}: {
  title: string;
  result: ComparisonResult;
}) {
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
