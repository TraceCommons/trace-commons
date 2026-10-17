import { useState } from "react";
import {
  useAutomaticGrantCopy,
  useContributorDisclosureCopy,
} from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { ContributionPath } from "../flow1";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { ButtonPrimary, GlassButton, Radio, RadioGroup } from "@/design-system";

// The path question, after the scope picker. No default: neither path is
// selected until the contributor picks one.
export function OnboardingPathStep({
  onboarding,
  busy,
  showPrivacy,
}: Pick<OnboardingStepProps, "onboarding" | "busy" | "showPrivacy">) {
  const core = useCoreStatus();
  const grantCopy = useAutomaticGrantCopy(core.scope, core.isSuccess);
  // Each path is named by its folder mode's one name (`folder_mode_labels`):
  // asking first is "notify_only", contributing automatically "auto_upload".
  const disclosure = useContributorDisclosureCopy();
  const modes = disclosure.data?.folder_mode_labels;
  const copy = modes ? grantCopy.data : undefined;
  const copyFailed = grantCopy.isError || disclosure.isError;
  const [path, setPath] = useState<ContributionPath | null>(null);
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        PATH
      </span>
      <h2 id="onboarding-path-heading">How should contributing work?</h2>
      {!copy && (
        <p
          className={`m-0 text-[12px] ${copyFailed ? "text-tc-outside" : "text-tc-secondary"}`}
          role={copyFailed ? "alert" : "status"}
        >
          {copyFailed
            ? "Path copy unavailable. Continue is disabled."
            : "Loading path copy…"}
        </p>
      )}
      <RadioGroup
        aria-labelledby="onboarding-path-heading"
        className="my-[18px] gap-px border-t border-tc-hairline"
        value={path ?? ""}
        onChange={(value) => setPath(value as ContributionPath)}
      >
        <label className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal">
          <Radio value="ask_first" disabled={busy || !copy} />
          <span>
            <strong>{modes?.notify_only}</strong>
            <small>{copy?.path_ask_first}</small>
          </span>
        </label>
        <label className="flex items-start gap-2.5 tc-hairline-bottom py-2 tc-label font-normal">
          <Radio value="automatic" disabled={busy || !copy} />
          <span>
            <strong>{modes?.auto_upload}</strong>
            <small>{copy?.path_automatic}</small>
          </span>
        </label>
      </RadioGroup>
      <div className="mt-3 flex flex-wrap gap-2">
        <GlassButton
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </GlassButton>
        <ButtonPrimary size="sm"
          type="button"
          onClick={() => {
            if (path) onboarding.choosePath(path, showPrivacy === true);
          }}
          disabled={busy || !copy || path === null || showPrivacy === null}
        >
          Continue
        </ButtonPrimary>
      </div>
    </section>
  );
}
