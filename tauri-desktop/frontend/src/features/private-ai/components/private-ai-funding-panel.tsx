import { Button } from "@/components/ui/button";
import type { usePrivateAi } from "../hooks/use-private-ai";

type PrivateAiController = ReturnType<typeof usePrivateAi>;

export function PrivateAiFundingPanel({
  privateAi,
}: {
  privateAi: PrivateAiController;
}) {
  const funding = privateAi.funding;
  const verifiedFundingUrl = privateAi.verifiedFundingUrl;
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            CLOUD BILLING
          </span>
          <h2>Manage credits</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void privateAi.refreshFunding()}
          disabled={privateAi.busy}
        >
          Refresh account
        </Button>
      </div>
      {funding ? (
        <>
          <p>{funding.message}</p>
          {funding.organizationName && (
            <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
              Organization: {funding.organizationName}
            </p>
          )}
          {funding.browserUrl && !verifiedFundingUrl && (
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={() => void privateAi.verifyFunding()}
              disabled={privateAi.busy}
            >
              Verify current account
            </Button>
          )}
          {verifiedFundingUrl && (
            <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
              <Button
                className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
                type="button"
                onClick={() => void privateAi.openBrowser(verifiedFundingUrl)}
                disabled={privateAi.busy}
              >
                Open verified billing
              </Button>
            </p>
          )}
        </>
      ) : (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Account destination not read.
        </p>
      )}
      <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
        Billing URL is released only after Rust verifies current organization
        and connection revision. This app never chooses a payer.
      </p>
    </section>
  );
}
