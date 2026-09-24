import type { UseFormReturn } from "react-hook-form";
import { Input } from "@/components/ui/input";
import { FormFieldError } from "../../../components/form-field-error";
import type { ComparisonSpecificationFormValues } from "../forms";

export function ComparisonSpecificationFields({
  form,
}: {
  form: UseFormReturn<ComparisonSpecificationFormValues>;
}) {
  const errors = form.formState.errors;
  return (
    <div className="mt-[22px] grid grid-cols-2 gap-4">
      <label htmlFor="spec-project">
        Project UUID
        <Input
          id="spec-project"
          {...form.register("projectId")}
          aria-invalid={Boolean(errors.projectId)}
          aria-describedby={errors.projectId ? "spec-project-error" : undefined}
        />
        <FormFieldError
          id="spec-project-error"
          message={errors.projectId?.message}
        />
      </label>
      <label htmlFor="spec-language">
        Language
        <Input
          id="spec-language"
          {...form.register("language")}
          aria-invalid={Boolean(errors.language)}
          aria-describedby={errors.language ? "spec-language-error" : undefined}
        />
        <FormFieldError
          id="spec-language-error"
          message={errors.language?.message}
        />
      </label>
      <label htmlFor="spec-fingerprint">
        Configuration fingerprint
        <Input
          id="spec-fingerprint"
          {...form.register("fingerprint")}
          aria-invalid={Boolean(errors.fingerprint)}
          aria-describedby={
            errors.fingerprint ? "spec-fingerprint-error" : undefined
          }
        />
        <FormFieldError
          id="spec-fingerprint-error"
          message={errors.fingerprint?.message}
        />
      </label>
      <label htmlFor="spec-cohorts">
        Cohorts
        <Input
          id="spec-cohorts"
          {...form.register("cohorts")}
          placeholder="model-a, model-b"
          aria-invalid={Boolean(errors.cohorts)}
          aria-describedby={errors.cohorts ? "spec-cohorts-error" : undefined}
        />
        <FormFieldError
          id="spec-cohorts-error"
          message={errors.cohorts?.message}
        />
      </label>
      <label htmlFor="spec-start">
        Start date
        <Input
          id="spec-start"
          {...form.register("dateStart")}
          type="date"
          aria-invalid={Boolean(errors.dateStart)}
          aria-describedby={errors.dateStart ? "spec-start-error" : undefined}
        />
        <FormFieldError
          id="spec-start-error"
          message={errors.dateStart?.message}
        />
      </label>
      <label htmlFor="spec-end">
        End date
        <Input
          id="spec-end"
          {...form.register("dateEnd")}
          type="date"
          aria-invalid={Boolean(errors.dateEnd)}
          aria-describedby={errors.dateEnd ? "spec-end-error" : undefined}
        />
        <FormFieldError id="spec-end-error" message={errors.dateEnd?.message} />
      </label>
      <label htmlFor="spec-cutoff">
        Evidence cutoff
        <Input
          id="spec-cutoff"
          {...form.register("cutoff")}
          type="datetime-local"
          aria-invalid={Boolean(errors.cutoff)}
          aria-describedby={errors.cutoff ? "spec-cutoff-error" : undefined}
        />
        <FormFieldError
          id="spec-cutoff-error"
          message={errors.cutoff?.message}
        />
      </label>
    </div>
  );
}
