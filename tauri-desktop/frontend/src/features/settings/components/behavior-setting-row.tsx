import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { BehaviorSetting } from "../api/behavior-api";
import { type BehaviorFormValues, behaviorFormSchema } from "../forms";

export function BehaviorSettingRow({
  label,
  detail,
  setting,
  value,
  min,
  max,
  unit,
  busy,
  onSave,
}: {
  label: string;
  detail: string;
  setting: BehaviorSetting;
  value: number;
  min: number;
  max: number;
  unit: string;
  busy: boolean;
  onSave: (setting: BehaviorSetting, value: number) => Promise<unknown>;
}) {
  const form = useForm<BehaviorFormValues>({
    resolver: zodResolver(behaviorFormSchema(min, max)),
    defaultValues: { value: String(value) },
    mode: "onChange",
  });
  useEffect(() => {
    if (!form.formState.isDirty) form.reset({ value: String(value) });
  }, [form, value]);
  const error = form.formState.errors.value?.message;
  const submit = async (values: BehaviorFormValues) => {
    const next = Number(values.value);
    try {
      await onSave(setting, next);
      form.reset({ value: String(next) });
    } catch {
      form.reset({ value: String(value) });
    }
  };
  return (
    <form
      className="flex items-center justify-between gap-5 border-b border-border py-[15px]"
      onSubmit={form.handleSubmit(submit)}
    >
      <div>
        <strong>{label}</strong>
        <span>{detail}</span>
      </div>
      <div className="flex items-start gap-2">
        <label>
          <Input
            {...form.register("value")}
            type="number"
            min={min}
            max={max}
            step="1"
            disabled={busy}
            aria-invalid={Boolean(error)}
            aria-describedby={error ? `${setting}-error` : undefined}
          />
          <span>{unit}</span>
          <FormFieldError id={`${setting}-error`} message={error} />
        </label>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="submit"
          disabled={busy || !form.formState.isValid}
        >
          Save
        </Button>
      </div>
    </form>
  );
}
