import type { usePrivateAi } from "../hooks/use-private-ai";
import { GlassButton, TertiaryLink } from "@/design-system";

type PrivateAiController = ReturnType<typeof usePrivateAi>;

export function PrivateAiFundingPanel({
  privateAi,
}: {
  privateAi: PrivateAiController;
}) {
  const funding = privateAi.funding;
  const verifiedFundingUrl = privateAi.verifiedFundingUrl;
  return (
    <section className="tc-card mb-2.5">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            CLOUD BILLING
          </span>
          <h2>Manage credits</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void privateAi.refreshFunding()}
          disabled={privateAi.busy}
        >
          Refresh account
        </TertiaryLink>
      </div>
      {funding ? (
        <>
          <p>{funding.message}</p>
          {funding.organizationName && (
            <p className="m-0 tc-caption tc-text-tertiary">
              Organization: {funding.organizationName}
            </p>
          )}
          {funding.browserUrl && !verifiedFundingUrl && (
            <GlassButton
              type="button"
              onClick={() => void privateAi.verifyFunding()}
              disabled={privateAi.busy}
            >
              Verify current account
            </GlassButton>
          )}
          {verifiedFundingUrl && (
            <p className="m-0 tc-caption tc-text-tertiary">
              <TertiaryLink
                type="button"
                onClick={() => void privateAi.openBrowser(verifiedFundingUrl)}
                disabled={privateAi.busy}
              >
                Open verified billing
              </TertiaryLink>
            </p>
          )}
        </>
      ) : (
        <p className="mt-3 mb-1 tc-body tc-text-tertiary">
          Account destination not read.
        </p>
      )}
      <p className="m-0 tc-caption tc-text-tertiary">
        Billing URL is released only after Rust verifies current organization
        and connection revision. This app never chooses a payer.
      </p>
    </section>
  );
}
