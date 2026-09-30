import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useMemo } from "react";
import { useController, useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import type { ConsentOption } from "../../onboarding/public";
import {
  type ConsentSettingsFormValues,
  consentSettingsFormSchema,
} from "../forms";
import { Checkbox, TertiaryLink } from "@/design-system";

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
    <section className="tc-card">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            CONSENT
          </span>
          <h2>How may your traces be used?</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy}
        >
          Refresh
        </TertiaryLink>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Applies to traces sent from now on. Always-included scope cannot be
        removed; every optional scope is changed from the daemon's current
        answer.
      </p>
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Loading consent options…
        </p>
      )}
      {groups.map(
        (group) =>
          group.values.length > 0 && (
            <div
              className="mt-3 grid gap-px"
              key={group.title}
            >
              <span className="mb-1.5 block tc-eyebrow">
                {group.title}
              </span>
              {group.values.map((option) => {
                const checked = scopes.field.value.includes(option.name);
                return (
                  <label
                    className="flex items-start gap-2.5 border-b border-tc-hairline py-3 text-[12px] font-normal text-[var(--tc-text-primary)]"
                    htmlFor={`settings-consent-${option.name}`}
                    key={option.name}
                  >
                    <Checkbox
                      id={`settings-consent-${option.name}`}
                      checked={checked || option.always_on}
                      disabled={busy || option.always_on}
                      onChange={() => toggle(option.name)}
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
