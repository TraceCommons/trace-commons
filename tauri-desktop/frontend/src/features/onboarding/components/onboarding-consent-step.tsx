import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useMemo } from "react";
import { useController, useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { type ConsentFormValues, consentFormSchema } from "../forms";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingConsentStep({
  onboarding,
  settings,
  alreadyEnrolled,
  busy,
  showPrivacy,
}: Pick<
  OnboardingStepProps,
  "onboarding" | "settings" | "alreadyEnrolled" | "busy" | "showPrivacy"
>) {
  const alwaysOn = useMemo(
    () =>
      onboarding.options
        .filter((option) => option.always_on)
        .map((option) => option.name),
    [onboarding.options],
  );
  const form = useForm<ConsentFormValues>({
    resolver: zodResolver(consentFormSchema(alwaysOn)),
    defaultValues: { scopes: alwaysOn },
  });
  const scopes = useController({ control: form.control, name: "scopes" });
  useEffect(() => {
    if (!form.formState.isDirty) form.reset({ scopes: alwaysOn });
  }, [alwaysOn, form]);
  const scopeError = scopes.fieldState.error?.message;
  const toggle = (name: string) => {
    const current = scopes.field.value ?? [];
    scopes.field.onChange(
      current.includes(name)
        ? current.filter((value) => value !== name)
        : [...current, name],
    );
  };
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        CONSENT
      </span>
      <h2>How may your traces be used?</h2>
      <form
        onSubmit={form.handleSubmit(
          (values) => void onboarding.saveConsent(values.scopes, showPrivacy === true),
        )}
      >
        <div className="my-[18px] grid gap-px border-t border-border">
          {onboarding.options.map((option) => (
            <label
              className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
              key={option.name}
            >
              <Checkbox
                checked={scopes.field.value.includes(option.name)}
                onCheckedChange={() => toggle(option.name)}
                disabled={busy || option.always_on}
                aria-invalid={Boolean(scopeError)}
                aria-describedby={
                  scopeError ? "consent-scopes-error" : undefined
                }
              />
              <span>
                <strong>
                  {option.name.replaceAll("_", " ")}
                  {option.always_on
                    ? " · required"
                    : option.grants_data_use
                      ? " · data use"
                      : " · attribution only"}
                </strong>
                <small>{option.description}</small>
              </span>
            </label>
          ))}
        </div>
        <FormFieldError id="consent-scopes-error" message={scopeError} />
        {onboarding.state === "loading" && (
          <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
            Loading consent options…
          </p>
        )}
        {showPrivacy === null && (
          <div className="grid gap-2 text-sm text-destructive" role="alert">
            <p>Privacy settings are unavailable. Refresh before continuing.</p>
            <Button
              type="button"
              variant="outline"
              onClick={() => void settings.refresh()}
              disabled={settings.isFetching}
            >
              {settings.isFetching ? "Refreshing…" : "Refresh privacy settings"}
            </Button>
          </div>
        )}
        <div className="mt-6 flex gap-2.5">
          {!alreadyEnrolled && (
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={onboarding.back}
              disabled={busy}
            >
              Back
            </Button>
          )}
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="submit"
            disabled={
              busy ||
              onboarding.options.length === 0 ||
              settings.state === "loading" ||
              showPrivacy === null
            }
          >
            Continue
          </Button>
        </div>
      </form>
    </section>
  );
}
