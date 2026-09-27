import type { ComparisonTaskDetail } from "../comparisons";
import type {
  ComparisonResult,
  ComparisonSpecification,
  ComparisonSpecificationInput,
} from "../specifications";
import { ComparisonSpecificationForm } from "./comparison-specification-form";
import { ComparisonSpecificationList } from "./comparison-specification-list";
import { ComparisonSpecificationResult } from "./comparison-specification-result";

type SpecificationController = {
  specifications: ComparisonSpecification[];
  preview: { result: ComparisonResult } | null;
  result: ComparisonResult | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  calculate: (input: ComparisonSpecificationInput) => Promise<void>;
  save: (input: ComparisonSpecificationInput) => Promise<void>;
  evaluate: (id: string) => Promise<void>;
};

export function ComparisonSpecificationsPanel({
  tasks,
  specifications,
}: {
  tasks: ComparisonTaskDetail[];
  specifications: SpecificationController;
}) {
  const busy =
    specifications.state === "busy" || specifications.state === "loading";

  return (
    <section className="mb-4 rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            COMPARISON SPECIFICATIONS
          </span>
          <h2>Evaluate a fixed retrospective cohort</h2>
        </div>
        <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start">
          {specifications.specifications.length}
        </span>
      </div>
      <p>
        Specifications freeze date, cohort, and exact configuration inputs.
        Results remain descriptive; declared model cohorts are not verified
        identities.
      </p>
      {specifications.error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {specifications.error}
        </p>
      )}
      <ComparisonSpecificationForm
        tasks={tasks}
        busy={busy}
        onCalculate={specifications.calculate}
        onSave={specifications.save}
      />
      {specifications.preview && (
        <ComparisonSpecificationResult
          title="Preview result"
          result={specifications.preview.result}
        />
      )}
      <ComparisonSpecificationList
        specifications={specifications.specifications}
        busy={busy}
        onEvaluate={specifications.evaluate}
      />
      {specifications.result && (
        <ComparisonSpecificationResult
          title="Saved result"
          result={specifications.result}
        />
      )}
    </section>
  );
}
