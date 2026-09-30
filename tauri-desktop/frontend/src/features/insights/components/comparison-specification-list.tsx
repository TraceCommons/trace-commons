import type { ComparisonSpecification } from "../specifications";
import { TertiaryLink } from "@/design-system";

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
    <div className="mt-3 grid gap-px">
      {specifications.map((specification) => (
        <div
          className="grid grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3.5 border-b border-tc-hairline py-3.5"
          key={specification.id}
        >
          <span className="tc-tool-tile tc-tool-tile--lg tc-tool-tile--folder">
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
            <TertiaryLink
              className="max-[860px]:justify-self-start"
              type="button"
              onClick={() => void onEvaluate(specification.id)}
              disabled={busy}
            >
              Evaluate
            </TertiaryLink>
          </span>
        </div>
      ))}
    </div>
  );
}
