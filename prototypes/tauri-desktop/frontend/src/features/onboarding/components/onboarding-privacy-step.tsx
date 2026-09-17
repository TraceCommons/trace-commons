import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { zodResolver } from "@hookform/resolvers/zod";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { type PrivacyFormValues, privacyFormSchema } from "../forms";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingPrivacyStep({
  onboarding,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "busy">) {
  const form = useForm<PrivacyFormValues>({
    resolver: zodResolver(privacyFormSchema),
    defaultValues: { privacyChoice: "local" },
  });
  const privacyChoice = form.watch("privacyChoice");
  const choiceError = form.formState.errors.privacyChoice?.message;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        OPTIONAL THIRD-PARTY SCAN
      </span>
      <h2>Choose the boundary</h2>
      <p>
        Local scrubbing removes secrets, keys, tokens, and credentials either
        way. The optional NEAR AI scan sends message text, not tool output or
        file contents, to a third party before Trace Commons receives it. If it
        is unreachable, nothing is sent unscanned.
      </p>
      <form
        onSubmit={form.handleSubmit(
          (values) =>
            void onboarding.savePrivacy(values.privacyChoice === "scan"),
        )}
      >
        <RadioGroup
          className="my-[18px] gap-px border-t border-border"
          value={privacyChoice}
          onValueChange={(value) =>
            form.setValue("privacyChoice", value as PrivacyFormValues["privacyChoice"], {
              shouldDirty: true,
              shouldValidate: true,
            })
          }
        >
          <label className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground">
            <RadioGroupItem
              value="local"
              disabled={busy}
              aria-invalid={Boolean(choiceError)}
              aria-describedby={
                choiceError ? "privacy-choice-error" : undefined
              }
            />
            <span>
              <strong>Local scrubbing only</strong>
              <small>Keep message text on this machine.</small>
            </span>
          </label>
          <label className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground">
            <RadioGroupItem
              value="scan"
              disabled={busy}
              aria-invalid={Boolean(choiceError)}
              aria-describedby={
                choiceError ? "privacy-choice-error" : undefined
              }
            />
            <span>
              <strong>Local scrubbing + NEAR AI scan</strong>
              <small>Use the second scanner after this disclosure.</small>
            </span>
          </label>
        </RadioGroup>
        <FormFieldError id="privacy-choice-error" message={choiceError} />
        <div className="mt-6 flex gap-2.5">
          <Button
            className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
            type="button"
            onClick={onboarding.back}
            disabled={busy}
          >
            Back
          </Button>
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="submit"
            disabled={busy}
          >
            Continue
          </Button>
        </div>
      </form>
    </section>
  );
}
