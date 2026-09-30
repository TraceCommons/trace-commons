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
    <section className="mb-4 tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            COMPARISON SPECIFICATIONS
          </span>
          <h2>Evaluate a fixed retrospective cohort</h2>
        </div>
        <span className="tc-chip tc-chip--glass self-start">
          {specifications.specifications.length}
        </span>
      </div>
      <p>
        Specifications freeze date, cohort, and exact configuration inputs.
        Results remain descriptive; declared model cohorts are not verified
        identities.
      </p>
      {specifications.error && (
        <p className="tc-alert">
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
