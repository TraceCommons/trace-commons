import type { InsightSummary as InsightSummaryData } from "../types";

export function InsightSummary({ summary }: { summary: InsightSummaryData }) {
  const categoryText =
    summary.categories
      .map((item) => `${item.category} ${item.snapshots}`)
      .join(" · ") || "None yet";
  const outcomeText =
    summary.outcomes
      .map((item) => `${item.outcome} ${item.snapshots}`)
      .join(" · ") || "None yet";
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            SAVED HISTORY
          </span>
          <h2>Local analysis summary</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          Rust-provided
        </span>
      </div>
      <div className="mt-7 grid grid-cols-3 gap-px border-y border-border">
        <div>
          <span>Saved snapshots</span>
          <strong>{summary.saved_snapshots}</strong>
        </div>
        <div>
          <span>Assessed by you</span>
          <strong>{summary.assessed_snapshots}</strong>
        </div>
        <div>
          <span>Not assessed</span>
          <strong>{summary.unassessed_snapshots}</strong>
        </div>
      </div>
      <div className="mt-4 grid gap-[5px]">
        <p>
          <b>Categories:</b> {categoryText}
        </p>
        <p>
          <b>Outcomes:</b> {outcomeText}
        </p>
        <p>
          <b>Coverage:</b>{" "}
          {summary.metrics.length > 0
            ? `${summary.metrics.length} observed metric${summary.metrics.length === 1 ? "" : "s"}`
            : "No observed metrics"}
        </p>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Assessments are user-reported. Saved sessions are not independently
        verified tasks; unknown values are not zero.
      </p>
    </section>
  );
}
