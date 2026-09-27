import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { initialScopeSelection, scopeChoice } from "../flow1";
import type { OnboardingStepProps } from "./onboarding-step-types";

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
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        CONSENT
      </span>
      <h2>How may your traces be used?</h2>
      {copy ? (
        <p
          className="m-0 text-[12px] leading-[1.55] text-muted-foreground"
          id="onboarding-scope-required"
        >
          {copy.scope_required}
        </p>
      ) : (
        <p
          className={`m-0 text-[12px] ${grantCopy.isError ? "text-destructive" : "text-muted-foreground"}`}
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
        <div className="my-[18px] grid gap-px border-t border-border">
          {onboarding.options.map((option) => (
            <label
              className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground"
              key={option.name}
            >
              <Checkbox
                checked={selected.includes(option.name)}
                onCheckedChange={() => toggle(option.name)}
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
            className="m-0 text-xs text-destructive"
            id="onboarding-scope-missing"
            role="alert"
          >
            Tick every required scope to continue, or choose Decide later.
          </p>
        )}
        {onboarding.state === "loading" && (
          <p
            className="mt-[30px] mb-1 text-[13px] text-muted-foreground"
            role="status"
          >
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
            type="button"
            variant="outline"
            onClick={() => onboarding.declineScopes(showPrivacy === true)}
            disabled={busy || !privacyKnown}
          >
            Decide later
          </Button>
          <Button
            className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
            type="submit"
            disabled={
              busy || !copy || settings.state === "loading" || !privacyKnown
            }
          >
            Continue
          </Button>
        </div>
      </form>
    </section>
  );
}
