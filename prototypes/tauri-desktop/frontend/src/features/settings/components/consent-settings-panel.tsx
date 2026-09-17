import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useMemo } from "react";
import { useController, useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { ConsentOption } from "../../onboarding/types";
import {
  type ConsentSettingsFormValues,
  consentSettingsFormSchema,
} from "../forms";

export function ConsentSettingsPanel({
  options,
  granted,
  state,
  error,
  onRefresh,
  onToggle,
}: {
  options: ConsentOption[];
  granted: string[];
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  onRefresh: () => Promise<void>;
  onToggle: (scopes: string[]) => Promise<unknown>;
}) {
  const alwaysOn = useMemo(
    () =>
      options.filter((option) => option.always_on).map((option) => option.name),
    [options],
  );
  const authoritativeScopes = useMemo(
    () => normalizeScopes(granted, alwaysOn),
    [alwaysOn, granted],
  );
  const form = useForm<ConsentSettingsFormValues>({
    resolver: zodResolver(consentSettingsFormSchema(alwaysOn)),
    defaultValues: { scopes: normalizeScopes(granted, alwaysOn) },
  });
  const scopes = useController({ control: form.control, name: "scopes" });
  useEffect(() => {
    if (!form.formState.isDirty) form.reset({ scopes: authoritativeScopes });
  }, [authoritativeScopes, form]);
  const busy = state === "loading" || state === "busy";
  const scopeError = scopes.fieldState.error?.message;
  const groups = [
    {
      title: "Always included",
      values: options.filter((option) => option.always_on),
    },
    {
      title: "Optional data use",
      values: options.filter(
        (option) => !option.always_on && option.grants_data_use,
      ),
    },
    {
      title: "Credit",
      values: options.filter(
        (option) => !option.always_on && !option.grants_data_use,
      ),
    },
  ];
  const toggle = (name: string) => {
    const next = scopes.field.value.includes(name)
      ? scopes.field.value.filter((value) => value !== name)
      : [...scopes.field.value, name];
    for (const required of alwaysOn)
      if (!next.includes(required)) next.push(required);
    scopes.field.onChange(next);
    void onToggle(next)
      .then(() => form.reset({ scopes: next }))
      .catch(() => form.reset({ scopes: normalizeScopes(granted, alwaysOn) }));
  };
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            CONSENT
          </span>
          <h2>How may your traces be used?</h2>
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
        Applies to traces sent from now on. Always-included scope cannot be
        removed; every optional scope is changed from the daemon's current
        answer.
      </p>
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Loading consent options…
        </p>
      )}
      {groups.map(
        (group) =>
          group.values.length > 0 && (
            <div
              className="mt-5 grid gap-px border-t border-border"
              key={group.title}
            >
              <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
                {group.title}
              </span>
              {group.values.map((option) => {
                const checked = scopes.field.value.includes(option.name);
                return (
                  <label
                    className="flex items-start gap-2.5 border-b border-border py-3 text-[12px] font-normal text-foreground"
                    key={option.name}
                  >
                    <Checkbox
                      checked={checked || option.always_on}
                      disabled={busy || option.always_on}
                      onCheckedChange={() => toggle(option.name)}
                      aria-invalid={Boolean(scopeError)}
                      aria-describedby={
                        scopeError ? "settings-consent-error" : undefined
                      }
                    />
                    <span>
                      <strong>
                        {option.name.replaceAll("_", " ")}
                        {option.always_on ? " · required" : ""}
                      </strong>
                      <small>{option.description}</small>
                    </span>
                  </label>
                );
              })}
            </div>
          ),
      )}
      <FormFieldError id="settings-consent-error" message={scopeError} />
    </section>
  );
}

function normalizeScopes(granted: string[], alwaysOn: string[]) {
  return [...new Set([...granted, ...alwaysOn])];
}
