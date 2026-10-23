export type HistoryFilter =
  | "all"
  | "accepted"
  | "submitted"
  | "quarantined"
  | "withdrawn";

const options: Array<{ value: HistoryFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "accepted", label: "In commons" },
  { value: "submitted", label: "Waiting to be scored" },
  { value: "quarantined", label: "Privacy review" },
  { value: "withdrawn", label: "Withdrawn" },
];

export function HistoryFilterBar({
  value,
  counts,
  onChange,
}: {
  value: HistoryFilter;
  counts: Partial<Record<HistoryFilter, number>>;
  onChange: (value: HistoryFilter) => void;
}) {
  return (
    <fieldset
      className="m-0 flex flex-wrap gap-1 border-0 p-0"
      aria-label="Filter history"
    >
      <legend className="sr-only">Filter history</legend>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          className={`tc-btn rounded-full px-2.5 py-[5px] text-[11px] font-semibold ${
            value === option.value
              ? "bg-white/16 text-[var(--tc-text-primary)]"
              : "bg-transparent tc-text-tertiary hover:text-[var(--tc-text-primary)]"
          }`}
          onClick={() => onChange(option.value)}
        >
          {option.label}
          <span className="tc-tabular opacity-70">
            {counts[option.value] ?? 0}
          </span>
        </button>
      ))}
    </fieldset>
  );
}
