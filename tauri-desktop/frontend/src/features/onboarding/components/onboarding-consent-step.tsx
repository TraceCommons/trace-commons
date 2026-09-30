import { useState } from "react";
import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { initialScopeSelection, scopeChoice } from "../flow1";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { ButtonPrimary, Checkbox, GlassButton } from "@/design-system";

// R7's scope picker: immediately after connect, before the path question,
// with nothing selected -- the floor scope included -- so no grant ships at
// a scope nobody chose. Declining lands on Flow 2 with no grant.
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
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  const copy = grantCopy.data;
  const [selected, setSelected] = useState<string[]>(initialScopeSelection);
  const choice = scopeChoice(onboarding.options, selected);
  // Nothing is marked invalid until the contributor tries to continue: an
  // unticked required scope is the starting state, not a mistake.
  const [attempted, setAttempted] = useState(false);
  const showMissing = attempted && choice.missingRequired.length > 0;
  const toggle = (name: string) => {
    setSelected((current) =>
      current.includes(name)
        ? current.filter((value) => value !== name)
        : [...current, name],
    );
  };
  const privacyKnown = showPrivacy !== null;
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        CONSENT
      </span>
      <h2>How may your traces be used?</h2>
      {copy ? (
        <p
          className="m-0 tc-label font-normal leading-[17px] tc-text-secondary"
          id="onboarding-scope-required"
        >
          {copy.scope_required}
        </p>
      ) : (
        <p
          className={`m-0 text-[12px] ${grantCopy.isError ? "text-tc-outside" : "text-tc-secondary"}`}
          role={grantCopy.isError ? "alert" : "status"}
        >
          {grantCopy.isError
            ? "Consent copy unavailable. Continue is disabled."
            : "Loading consent copy…"}
        </p>
      )}
      <form
        onSubmit={(event) => {
          event.preventDefault();
          setAttempted(true);
          if (choice.canContinue && copy && privacyKnown)
            void onboarding.saveConsent(selected);
        }}
      >
        <div className="my-3 grid gap-px">
          {onboarding.options.map((option) => (
            <label
              className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal"
              key={option.name}
            >
              <Checkbox
                checked={selected.includes(option.name)}
                onChange={() => toggle(option.name)}
                disabled={busy}
                aria-invalid={
                  showMissing && choice.missingRequired.includes(option.name)
                }
                aria-describedby={
                  showMissing
                    ? "onboarding-scope-required onboarding-scope-missing"
                    : "onboarding-scope-required"
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
        {showMissing && (
          <p
            className="m-0 text-xs text-tc-outside"
            id="onboarding-scope-missing"
            role="alert"
          >
            Tick every required scope to continue, or choose Decide later.
          </p>
        )}
        {onboarding.state === "loading" && (
          <p
            className="mt-3 mb-1 tc-body tc-text-tertiary"
            role="status"
          >
            Loading consent options…
          </p>
        )}
        {showPrivacy === null && (
          <div className="grid gap-2 text-sm text-tc-outside" role="alert">
            <p>Privacy settings are unavailable. Refresh before continuing.</p>
            <GlassButton
              type="button"
              onClick={() => void settings.refresh()}
              disabled={settings.isFetching}
            >
              {settings.isFetching ? "Refreshing…" : "Refresh privacy settings"}
            </GlassButton>
          </div>
        )}
        <div className="mt-3 flex flex-wrap gap-2">
          {!alreadyEnrolled && (
            <GlassButton
              type="button"
              onClick={onboarding.back}
              disabled={busy}
            >
              Back
            </GlassButton>
          )}
          <GlassButton
            type="button"
            onClick={() => onboarding.declineScopes(showPrivacy === true)}
            disabled={busy || !privacyKnown}
          >
            Decide later
          </GlassButton>
          <ButtonPrimary size="sm"
            type="submit"
            disabled={
              busy || !copy || settings.state === "loading" || !privacyKnown
            }
          >
            Continue
          </ButtonPrimary>
        </div>
      </form>
    </section>
  );
}
