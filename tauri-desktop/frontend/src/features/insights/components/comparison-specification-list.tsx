import { Button } from "@/components/ui/button";
import type { ComparisonSpecification } from "../specifications";

export function ComparisonSpecificationList({
  specifications,
  busy,
  onEvaluate,
}: {
  specifications: ComparisonSpecification[];
  busy: boolean;
  onEvaluate: (id: string) => Promise<void>;
}) {
  return (
    <div className="mt-[22px] grid gap-px border-t border-border">
      {specifications.map((specification) => (
        <div
          className="grid grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-b border-border py-3.5"
          key={specification.id}
        >
          <span className="grid h-[34px] w-[34px] place-items-center rounded-[9px] bg-primary text-[12px] font-extrabold text-primary-foreground bg-blue">
            S
          </span>
          <span className="grid min-w-0 gap-1">
            <strong>
              {specification.date_start} – {specification.date_end}
            </strong>
            <span>
              {specification.cohort_labels.join(" / ")} · {specification.id}
            </span>
          </span>
          <span className="grid min-w-[116px] gap-1 text-right max-[860px]:min-w-0 max-[860px]:text-left">
            <strong>{specification.estimator_state.status}</strong>
            <Button
              className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary max-[860px]:justify-self-start"
              type="button"
              onClick={() => void onEvaluate(specification.id)}
              disabled={busy}
            >
              Evaluate
            </Button>
          </span>
        </div>
      ))}
    </div>
  );
}
