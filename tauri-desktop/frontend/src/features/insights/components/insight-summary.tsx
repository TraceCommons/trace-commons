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
    <section className="tc-card mb-2.5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            SAVED HISTORY
          </span>
          <h2>Local analysis summary</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
          Rust-provided
        </span>
      </div>
      <div className="mt-7 grid grid-cols-3 gap-px border-y border-tc-hairline">
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
      <p className="m-0 tc-caption tc-text-tertiary">
        Assessments are user-reported. Saved traces are not independently
        verified tasks; unknown values are not zero.
      </p>
    </section>
  );
}
