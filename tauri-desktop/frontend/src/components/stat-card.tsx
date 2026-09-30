type StatCardProps = {
  label: string;
  value: string;
  /**
   * Accepted for the views the Monitor does not host (Insights, Compute);
   * the design's stat tile shows label and value only.
   */
  detail?: string;
  tone?: "green" | "blue" | "gold";
};

/** Stat tile: eyebrow label over an 18px tabular value. */
export function StatCard({ label, value }: StatCardProps) {
  return (
    <div className="tc-card min-w-0" style={{ padding: "10px 12px" }}>
      <div className="tc-eyebrow">{label}</div>
      <div className="truncate text-[18px] font-bold leading-6 tc-tabular">
        {value}
      </div>
    </div>
  );
}
