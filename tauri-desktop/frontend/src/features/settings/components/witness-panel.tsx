import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { FormFieldError } from "../../../components/form-field-error";
import type { WitnessStatus } from "../api/witness-api";
import { type WitnessFormValues, witnessFormSchema } from "../forms";

export function WitnessPanel({
  data,
  state,
  error,
  onRefresh,
  onConfigure,
  onClear,
}: {
  data: WitnessStatus | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  onRefresh: () => Promise<void>;
  onConfigure: (
    url: string,
    signingAddress: string,
    measurements: string[],
  ) => Promise<unknown>;
  onClear: () => Promise<unknown>;
}) {
  const form = useForm<WitnessFormValues>({
    resolver: zodResolver(witnessFormSchema),
    defaultValues: { url: "", signingAddress: "", measurements: "" },
    mode: "onChange",
  });
  const { isDirty } = form.formState;
  const { reset } = form;
  useEffect(() => {
    if (data && !isDirty) {
      reset({
        url: data.url ?? "",
        signingAddress: data.signing_address ?? "",
        measurements: data.pinned_measurements.join("\n"),
      });
    }
  }, [data, isDirty, reset]);
  const busy = state === "loading" || state === "busy";
  const configured = data?.url !== null && data?.url !== undefined;
  const errors = form.formState.errors;
  const save = async (values: WitnessFormValues) => {
    const measurements = values.measurements
      .split("\n")
      .map((entry) => entry.trim())
      .filter(Boolean);
    try {
      await onConfigure(values.url, values.signingAddress, measurements);
      form.reset(values);
    } catch {
      form.reset({
        url: data?.url ?? "",
        signingAddress: data?.signing_address ?? "",
        measurements: data?.pinned_measurements.join("\n") ?? "",
      });
    }
  };
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px] block">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            REDACTION WITNESS
          </span>
          <h2>Certificate boundary</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy}
        >
          Refresh
        </Button>
      </div>
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        A witness receives raw sessions only after local consent and measurement
        verification. A configured witness without a valid pin refuses
        submissions; no witness keeps local redaction.
      </p>
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {data && (
        <div
          className={`mt-5 grid gap-[6px] border-l-[3px] p-3.5 ${
            data.state.startsWith("refusing_") ||
            data.state === "settings_unreadable"
              ? "border-destructive/40 bg-destructive/10"
              : "border-primary bg-primary/10"
          }`}
        >
          <strong>{data.state_line}</strong>
          {data.pinned_measurement_line && (
            <span>{data.pinned_measurement_line}</span>
          )}
          {data.refusal && <small>Operator label: {data.refusal}</small>}
        </div>
      )}
      <form className="mt-5 grid gap-3.5" onSubmit={form.handleSubmit(save)}>
        <label htmlFor="witness-url">
          Witness URL
          <Input
            id="witness-url"
            {...form.register("url")}
            placeholder="https://witness.example"
            disabled={busy}
            aria-invalid={Boolean(errors.url)}
            aria-describedby={errors.url ? "witness-url-error" : undefined}
          />
          <FormFieldError
            id="witness-url-error"
            message={errors.url?.message}
          />
        </label>
        <label htmlFor="witness-address">
          Signing address
          <Input
            id="witness-address"
            {...form.register("signingAddress")}
            placeholder="0x…"
            disabled={busy}
            aria-invalid={Boolean(errors.signingAddress)}
            aria-describedby={
              errors.signingAddress ? "witness-address-error" : undefined
            }
          />
          <FormFieldError
            id="witness-address-error"
            message={errors.signingAddress?.message}
          />
        </label>
        <label htmlFor="witness-measurements">
          Expected measurements
          <Textarea
            id="witness-measurements"
            {...form.register("measurements")}
            rows={4}
            placeholder="mrtd=…\nmrtd=…,mrconfigid=…"
            disabled={busy}
            aria-invalid={Boolean(errors.measurements)}
            aria-describedby={
              errors.measurements ? "witness-measurements-error" : undefined
            }
          />
          <span>
            One measurement set per line. At least one valid pin required.
          </span>
          <FormFieldError
            id="witness-measurements-error"
            message={errors.measurements?.message}
          />
        </label>
        <div className="mt-2 flex gap-2.5">
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="submit"
            disabled={busy || !form.formState.isValid}
          >
            Save witness
          </Button>
          {configured && (
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary text-destructive"
              type="button"
              onClick={() => void onClear()}
              disabled={busy}
            >
              Return to local redaction
            </Button>
          )}
        </div>
      </form>
    </section>
  );
}
