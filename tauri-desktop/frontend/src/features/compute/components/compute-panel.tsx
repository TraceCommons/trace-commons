import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { type ComputeFormValues, computeFormSchema } from "../forms";
import type { useComputeStatus } from "../hooks/use-compute-status";
import { ButtonPrimary, GlassButton, Input } from "@/design-system";

/**
 * Compute consent and its worker: enable with an allowance, resume, pause,
 * and disable. Every label is the core's (`snapshot.copy`). Rendered by the
 * Compute page and by the Settings modal's Compute section, so a consent
 * granted in any build can always be paused and withdrawn.
 */
export function ComputePanel({
  compute,
}: {
  compute: ReturnType<typeof useComputeStatus>;
}) {
  const snapshot = compute.data;
  const form = useForm<ComputeFormValues>({
    resolver: zodResolver(computeFormSchema),
    defaultValues: { allowance: "4" },
    mode: "onChange",
  });
  const allowance = snapshot?.ram_allowance_gib;
  useEffect(() => {
    if (
      allowance !== null &&
      allowance !== undefined &&
      !form.formState.isDirty
    ) {
      form.reset({ allowance: String(allowance) });
    }
  }, [allowance, form]);
  const allowanceError = form.formState.errors.allowance?.message;
  return (
    <>
      {compute.state === "error" && (
        <p className="tc-alert">
          {compute.error}
        </p>
      )}
      {snapshot && (
        <section className="tc-card mb-2.5">
          <span className="mb-1.5 block tc-eyebrow">
            COMPUTE RESOURCE
          </span>
          <h2>{snapshot.title}</h2>
          <p>{snapshot.copy.introduction}</p>
          <p className="m-0 tc-caption tc-text-tertiary">
            {snapshot.detail}
          </p>
          <form
            onSubmit={form.handleSubmit(
              (values) =>
                void compute.command(
                  "enable_compute",
                  Number(values.allowance),
                ),
            )}
          >
            <label className="mt-[22px] grid max-w-[340px] gap-2 text-[11px] font-bold text-tc-secondary">
              {snapshot.copy.allowance_label}
              <Input
                {...form.register("allowance")}
                type="number"
                min="1"
                step="1"
                disabled={snapshot.consent_granted || compute.state === "busy"}
                aria-invalid={Boolean(allowanceError)}
                aria-describedby={
                  allowanceError ? "allowance-error" : undefined
                }
              />
              <span>{snapshot.copy.allowance_detail}</span>
              <FormFieldError id="allowance-error" message={allowanceError} />
            </label>
            <div className="mt-3 flex flex-wrap gap-2">
              <ButtonPrimary size="sm"
                type="submit"
                disabled={
                  compute.state === "busy" ||
                  snapshot.consent_granted ||
                  !snapshot.can_enable ||
                  !form.formState.isValid
                }
              >
                {snapshot.copy.enable}
              </ButtonPrimary>
              <GlassButton
                type="button"
                onClick={() => void compute.command("resume_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.resume}
              </GlassButton>
              <GlassButton
                type="button"
                onClick={() => void compute.command("pause_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.pause}
              </GlassButton>
              <GlassButton
                className="tc-text-outside"
                type="button"
                onClick={() => void compute.command("disable_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.disable}
              </GlassButton>
            </div>
          </form>
        </section>
      )}
    </>
  );
}
