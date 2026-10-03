import { useState } from "react";
import { useInferenceConnectionCopy } from "../../../lib/tauri/use-contributor-copy";
import { useOnboardingInference } from "../hooks/use-onboarding-inference";
import type { OnboardingStepProps } from "./onboarding-step-types";
import { GlassButton, Radio, RadioGroup } from "@/design-system";

// Connecting inference (K12): optional, and one route to a witness. Every
// sentence is the core's (`consent_copy::inference_connection_copy`, and an
// offer's disclosure picked per version). Nothing is chosen for the
// contributor: no offer is pre-selected, choosing records it on the account
// and installs nothing, and installing is a separate confirmation that says
// first what it stops. Skipping, the non-consequential action, comes before
// every action that changes something, and none of them is accented.
export function OnboardingInferenceStep({
  onboarding,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "busy">) {
  const copyQuery = useInferenceConnectionCopy();
  const copy = copyQuery.data;
  const inference = useOnboardingInference();
  const [chosen, setChosen] = useState<string | null>(null);
  const working = busy || inference.busy;
  const view = inference.view;
  const skip = (
    <GlassButton
      type="button"
      onClick={onboarding.finishInference}
      disabled={working}
    >
      {view.kind === "installed" || view.kind === "none" ? "Continue" : "Skip"}
    </GlassButton>
  );
  return (
    <section className="tc-card mb-2.5">
      <span className="mb-1.5 block tc-eyebrow">
        CONNECT INFERENCE (OPTIONAL)
      </span>
      <h2 id="onboarding-inference-heading">Connect inference?</h2>
      {!copy ? (
        <p
          className={`m-0 text-[12px] ${copyQuery.isError ? "text-tc-outside" : "text-tc-secondary"}`}
          role={copyQuery.isError ? "alert" : "status"}
        >
          {copyQuery.isError
            ? "This step's wording could not be loaded. You can skip it."
            : "Loading…"}
        </p>
      ) : (
        <div className="grid gap-3 text-[12px]">
          <p className="m-0">{copy.why}</p>
          <p className="m-0">{copy.grants_nothing}</p>
          {view.kind === "sign_in" && (
            <>
              <p className="m-0">{copy.sign_in}</p>
              {inference.signInFailed && (
                <p className="m-0 text-tc-outside" role="alert">
                  {copy.sign_in_failed}
                </p>
              )}
            </>
          )}
          {view.kind === "loading" && inference.failed && (
            <p className="m-0 text-tc-outside" role="alert">
              {copy.load_failed}
            </p>
          )}
          {view.kind === "none" && <p className="m-0">{copy.none_offered}</p>}
          {view.kind === "choose" && (
            <>
              {view.reselect && <p className="m-0">{copy.reselect}</p>}
              {view.otherDevice ? (
                <p className="m-0 font-bold">{copy.other_device}</p>
              ) : (
                <p className="m-0">{copy.one_device}</p>
              )}
              <RadioGroup
                aria-labelledby="onboarding-inference-heading"
                className="gap-px border-t border-tc-hairline"
                value={chosen ?? ""}
                onChange={(value) => setChosen(value as string)}
              >
                {view.offers.map((offer) => (
                  <label
                    key={offer.offer_id}
                    className="flex items-start gap-2.5 border-b border-tc-hairline py-2.5 font-normal text-[var(--tc-text-primary)]"
                  >
                    <Radio value={offer.offer_id} disabled={working} />
                    <span>
                      <strong>{offer.provider_id}</strong>
                      <small className="block">{offer.disclosure}</small>
                    </span>
                  </label>
                ))}
              </RadioGroup>
              {inference.selectFailed && (
                <p className="m-0 text-tc-outside" role="alert">
                  {copy.select_failed}
                </p>
              )}
            </>
          )}
          {view.kind === "install" && (
            <>
              {inference.previousRemoved && (
                <p className="m-0">{copy.previous_removed}</p>
              )}
              <p className="m-0 font-bold">{copy.install}</p>
              {inference.installFailed && (
                <p className="m-0 text-tc-outside" role="alert">
                  {copy.install_failed}
                </p>
              )}
            </>
          )}
          {view.kind === "installed" && (
            <>
              <p className="m-0" role="status">
                {copy.installed}
              </p>
              <p className="m-0 text-tc-secondary">{copy.disconnect}</p>
            </>
          )}
          {inference.disconnectPending && (
            <p className="m-0" role="status">
              {copy.disconnect_pending}
            </p>
          )}
        </div>
      )}
      <div className="mt-6 flex flex-wrap gap-2.5">
        <GlassButton
          type="button"
          onClick={onboarding.back}
          disabled={working}
        >
          Back
        </GlassButton>
        {skip}
        {copy && view.kind === "sign_in" && (
          <GlassButton
            type="button"
            onClick={inference.signIn}
            disabled={working}
          >
            Sign in
          </GlassButton>
        )}
        {copy && inference.signInUrl && (
          <GlassButton
            type="button"
            onClick={inference.openSignInUrl}
          >
            Open sign-in page
          </GlassButton>
        )}
        {copy && view.kind === "loading" && inference.failed && (
          <GlassButton
            type="button"
            onClick={inference.retry}
            disabled={working}
          >
            Try again
          </GlassButton>
        )}
        {copy && view.kind === "choose" && (
          <GlassButton
            type="button"
            onClick={() => {
              const offer = view.offers.find((o) => o.offer_id === chosen);
              if (offer) inference.select(offer, view.expectedVersion);
            }}
            disabled={working || chosen === null}
          >
            Connect
          </GlassButton>
        )}
        {copy && view.kind === "install" && (
          <GlassButton
            type="button"
            onClick={() => inference.install(view.target)}
            disabled={working}
          >
            Use this witness on this device
          </GlassButton>
        )}
        {copy && view.kind === "installed" && (
          <GlassButton
            type="button"
            onClick={() => inference.disconnect(view.connectionId)}
            disabled={working}
          >
            Disconnect
          </GlassButton>
        )}
      </div>
    </section>
  );
}
