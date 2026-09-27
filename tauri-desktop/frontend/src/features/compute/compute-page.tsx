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
    <div className="mx-auto max-w-[1080px] px-4 pb-12 pt-8 sm:px-8 sm:pb-16 sm:pt-10 lg:px-16 lg:pt-14">
      <PageHeader
        eyebrow="LOCAL / CAPACITY"
        title="Compute"
        description="Use local capacity for private model work when it is available and explicitly enabled."
        phase="PHASE 4"
      />
      <div className="mb-4 grid grid-cols-3 gap-3 max-[860px]:grid-cols-1">
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
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {compute.error}
        </p>
      )}
      {snapshot && (
        <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            COMPUTE RESOURCE
          </span>
          <h2>{snapshot.title}</h2>
          <p>{snapshot.copy.introduction}</p>
          <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
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
            <div className="mt-6 flex gap-2.5">
              <Button
                className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
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
                className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
                type="button"
                onClick={() => void compute.command("resume_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.resume}
              </Button>
              <Button
                className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
                type="button"
                onClick={() => void compute.command("pause_compute")}
                disabled={compute.state === "busy" || !snapshot.consent_granted}
              >
                {snapshot.copy.pause}
              </Button>
              <Button
                className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:bg-primary/80 text-destructive"
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
