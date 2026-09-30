import { Button } from "@/components/ui/button";
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
    <fieldset className="mt-5 flex flex-wrap gap-[7px]">
      <legend className="absolute h-px w-px overflow-hidden whitespace-nowrap [clip:rect(0_0_0_0)] [clip-path:inset(50%)]">
        Filter history
      </legend>
      {options.map((option) => (
        <Button
          key={option.value}
          className={value === option.value ? "history-filter-active" : ""}
          type="button"
          onClick={() => onChange(option.value)}
        >
          {option.label}
          <span>{counts[option.value] ?? 0}</span>
        </Button>
      ))}
    </fieldset>
  );
}
