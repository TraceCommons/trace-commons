import { useState } from "react";
import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useAutomaticGrantCopy } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { ContributionPath } from "../flow1";
import type { OnboardingStepProps } from "./onboarding-step-types";

// The path question, after the scope picker. No default: neither path is
// selected until the contributor picks one.
export function OnboardingPathStep({
  onboarding,
  busy,
  showPrivacy,
}: Pick<OnboardingStepProps, "onboarding" | "busy" | "showPrivacy">) {
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  const copy = grantCopy.data;
  const [path, setPath] = useState<ContributionPath | null>(null);
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        PATH
      </span>
      <h2 id="onboarding-path-heading">How should contributing work?</h2>
      {!copy && (
        <p
          className={`m-0 text-[12px] ${grantCopy.isError ? "text-destructive" : "text-muted-foreground"}`}
          role={grantCopy.isError ? "alert" : "status"}
        >
          {grantCopy.isError
            ? "Path copy unavailable. Continue is disabled."
            : "Loading path copy…"}
        </p>
      )}
      <RadioGroup
        aria-labelledby="onboarding-path-heading"
        className="my-[18px] gap-px border-t border-border"
        value={path ?? ""}
        onValueChange={(value) => setPath(value as ContributionPath)}
      >
        <label className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal">
          <RadioGroupItem value="ask_first" disabled={busy || !copy} />
          <span>
            <strong>Ask me each time</strong>
            <small>{copy?.path_ask_first}</small>
          </span>
        </label>
        <label className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal">
          <RadioGroupItem value="automatic" disabled={busy || !copy} />
          <span>
            <strong>Contribute automatically</strong>
            <small>{copy?.path_automatic}</small>
          </span>
        </label>
      </RadioGroup>
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
          type="button"
          onClick={() => {
            if (path) onboarding.choosePath(path, showPrivacy === true);
          }}
          disabled={busy || !copy || path === null || showPrivacy === null}
        >
          Continue
        </Button>
      </div>
    </section>
  );
}
