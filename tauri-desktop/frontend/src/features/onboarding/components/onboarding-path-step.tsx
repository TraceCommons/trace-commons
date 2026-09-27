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
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
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
        <label className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground">
          <RadioGroupItem value="ask_first" disabled={busy || !copy} />
          <span>
            <strong>Ask me each time</strong>
            <small>{copy?.path_ask_first}</small>
          </span>
        </label>
        <label className="flex items-start gap-2.5 border-b border-border py-2.5 text-[12px] font-normal text-foreground">
          <RadioGroupItem value="automatic" disabled={busy || !copy} />
          <span>
            <strong>Contribute automatically</strong>
            <small>{copy?.path_automatic}</small>
          </span>
        </label>
      </RadioGroup>
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
