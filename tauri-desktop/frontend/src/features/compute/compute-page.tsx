import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../components/form-field-error";
import { PageHeader } from "../../components/page-header";
import { StatCard } from "../../components/stat-card";
import { type ComputeFormValues, computeFormSchema } from "./forms";
import { useComputeStatus } from "./hooks/use-compute-status";

export function ComputePage() {
  const compute = useComputeStatus();
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
    <div className="tc-page">
      <PageHeader
        eyebrow="LOCAL / CAPACITY"
        title="Compute"
        description="Use local capacity for private model work when it is available and explicitly enabled."
        phase="PHASE 4"
      />
      <div className="mb-2.5 grid grid-cols-3 gap-1.5">
        <StatCard
          label="State"
          value={snapshot?.state ?? "Unknown"}
          detail={snapshot?.reason ?? "Compute monitor not connected"}
          tone={snapshot?.state === "unavailable" ? "gold" : "blue"}
        />
        <StatCard
          label="Consent"
          value={snapshot?.consent_granted ? "Granted" : "Required"}
          detail="Separate from trace contribution"
        />
        <StatCard
          label="Worker"
          value={snapshot?.available ? "Available" : "Unavailable"}
          detail="No worker starts automatically"
          tone="gold"
        />
      </div>
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
            <label className="mt-[22px] grid max-w-[340px] gap-2 text-[11px] font-bold text-muted-foreground">
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
              <Button
                className="tc-btn tc-btn--primary tc-btn--sm"
                type="submit"
                disabled={
                  compute.state === "busy" ||
                  snapshot.consent_granted ||
                  !snapshot.can_enable ||
                  !form.formState.isValid
                }
              >
                {snapshot.copy.enable}
              </Button>
              <Button
                className="tc-btn tc-btn--glass"
                type="button"
                onClick={() => void compute.command("resume_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.resume}
              </Button>
              <Button
                className="tc-btn tc-btn--glass"
                type="button"
                onClick={() => void compute.command("pause_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.pause}
              </Button>
              <Button
                className="tc-card text-[11px] font-bold text-foreground hover:bg-primary/80 text-destructive"
                type="button"
                onClick={() => void compute.command("disable_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.disable}
              </Button>
            </div>
          </form>
        </section>
      )}
    </div>
  );
}
