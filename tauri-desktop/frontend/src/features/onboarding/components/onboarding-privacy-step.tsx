import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { zodResolver } from "@hookform/resolvers/zod";
import { useForm } from "react-hook-form";
import { FormFieldError } from "../../../components/form-field-error";
import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { type PrivacyFormValues, privacyFormSchema } from "../forms";
import type { OnboardingStepProps } from "./onboarding-step-types";

export function OnboardingPrivacyStep({
  onboarding,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "busy">) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data?.privacy_scan;
  const form = useForm<PrivacyFormValues>({
    resolver: zodResolver(privacyFormSchema),
    defaultValues: { privacyChoice: "local" },
  });
  const privacyChoice = form.watch("privacyChoice");
  const choiceError = form.formState.errors.privacyChoice?.message;
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        OPTIONAL THIRD-PARTY SCAN
      </span>
      <h2>{copy?.title ?? "Choose the boundary"}</h2>
      {copy ? (
        <>
          <p>{copy.local_always}</p>
          <p>{copy.offer}</p>
          <p>{copy.disclosure}</p>
        </>
      ) : (
        <p className="text-destructive" role="alert">
          {disclosure.isError
            ? "Privacy scan disclosure unavailable. Continue is disabled."
            : "Loading privacy scan disclosure…"}
        </p>
      )}
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
          <label className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal">
            <RadioGroupItem
              value="local"
              disabled={busy}
              aria-invalid={Boolean(choiceError)}
              aria-describedby={
                choiceError ? "privacy-choice-error" : undefined
              }
            />
            <span>
              <strong>{copy?.local_only}</strong>
            </span>
          </label>
          <label className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal">
            <RadioGroupItem
              value="scan"
              disabled={busy}
              aria-invalid={Boolean(choiceError)}
              aria-describedby={
                choiceError ? "privacy-choice-error" : undefined
              }
            />
            <span>
              <strong>{copy?.with_near}</strong>
            </span>
          </label>
        </RadioGroup>
        <FormFieldError id="privacy-choice-error" message={choiceError} />
        <div className="mt-3 flex flex-wrap gap-2">
          <Button
            className="tc-btn tc-btn--glass"
            type="button"
            onClick={onboarding.back}
            disabled={busy}
          >
            Back
          </Button>
          <Button
            className="tc-btn tc-btn--primary tc-btn--sm"
            type="submit"
            disabled={busy || !copy}
          >
            Continue
          </Button>
        </div>
      </form>
    </section>
  );
}
