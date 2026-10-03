import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import type { useOnboardingNearAi } from "../hooks/use-onboarding-near-ai";
import { ButtonPrimary, GlassButton, Input, Select, TertiaryLink } from "@/design-system";

export function OnboardingNearAiJoin({
  nearAi,
  blocked = false,
}: {
  nearAi: ReturnType<typeof useOnboardingNearAi>;
  blocked?: boolean;
}) {
  const disclosures = useContributorDisclosureCopy();
  const disclosure = disclosures.data?.near_ai_enroll;
  return (
    <section className="grid gap-4 border-t border-tc-hairline pt-5">
      <div>
        <span className="mb-1.5 block tc-eyebrow">
          NEAR AI
        </span>
        <h3>{disclosure?.title ?? "Join with NEAR AI"}</h3>
        <p className="m-0 tc-label font-normal leading-[17px] tc-text-secondary">
          {disclosure?.what ?? "Loading NEAR AI enrollment disclosure…"}
        </p>
      </div>
      <div className="flex flex-col gap-4">
        <div className="tc-field">
          <label className="tc-label" htmlFor="near-ai-commons">Commons URL</label>
          <Input
            id="near-ai-commons"
            value={nearAi.commons}
            onChange={(event) => nearAi.setCommons(event.target.value)}
            placeholder="https://commons.example"
            disabled={nearAi.busy || blocked}
          />
          <p className="m-0 tc-caption tc-text-tertiary">Used only when you press Join.</p>
        </div>
      </div>
      {nearAi.credential.isPending && (
        <p className="m-0 tc-label font-normal tc-text-secondary">
          Checking NEAR AI sign-in status…
        </p>
      )}
      {nearAi.signedIn ? (
        <ButtonPrimary size="sm"
          type="button"
          onClick={() => nearAi.join.mutate()}
          disabled={
            nearAi.busy || blocked || !nearAi.commons.trim() || !disclosure
          }
        >
          {nearAi.join.isPending
            ? "Joining…"
            : (disclosure?.action ?? "Join with NEAR AI")}
        </ButtonPrimary>
      ) : (
        <div className="grid gap-3">
          <p className="m-0 tc-label font-normal tc-text-secondary">
            {disclosure?.needs_login ?? "Sign in disclosure unavailable."}
          </p>
          {disclosures.data ? (
            <div className="tc-card tc-card--quiet grid gap-2 text-[11px] leading-[1.55] text-tc-secondary">
              <p className="m-0 whitespace-pre-line">
                {disclosures.data.credential_cost}
              </p>
              {nearAi.provider === "near" && (
                <p className="m-0">
                  {disclosures.data.credential_wallet_notice}
                </p>
              )}
            </div>
          ) : (
            <p className="m-0 text-[11px] text-tc-outside">
              {disclosures.isError
                ? "Credential disclosure unavailable. Sign-in is disabled."
                : "Loading credential disclosure…"}
            </p>
          )}
          <div className="flex flex-wrap items-end gap-2.5">
            <div className="tc-field">
              <label className="tc-label" htmlFor="near-ai-provider">Provider</label>
              <Select
                id="near-ai-provider"
                value={nearAi.provider}
                onChange={(event) => nearAi.setProvider(event.target.value)}
                disabled={nearAi.busy || blocked}
              >
                <option value="github">GitHub</option>
                <option value="google">Google</option>
                <option value="near">NEAR wallet</option>
              </Select>
            </div>
            <GlassButton
              type="button"
              onClick={() => nearAi.start.mutate()}
              disabled={
                nearAi.busy ||
                blocked ||
                !disclosure ||
                !disclosures.data?.credential_cost ||
                (nearAi.provider === "near" &&
                  !disclosures.data?.credential_wallet_notice)
              }
            >
              {nearAi.start.isPending ? "Starting…" : "Start sign-in"}
            </GlassButton>
          </div>
          {nearAi.browserUrl && (
            <p className="m-0 tc-label font-normal tc-text-secondary">
              Open sign-in:{" "}
              <TertiaryLink
                type="button"
                className="h-auto p-0 text-[12px]"
                onClick={() => nearAi.open.mutate(nearAi.browserUrl ?? "")}
                disabled={nearAi.busy || blocked}
              >
                continue in browser
              </TertiaryLink>
              .
            </p>
          )}
        </div>
      )}
      {nearAi.error && (
        <p className="tc-alert m-0">
          {nearAi.error}
        </p>
      )}
    </section>
  );
}
