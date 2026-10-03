import { useState } from "react";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useArmingOfferCopy } from "../../../lib/tauri/use-contributor-copy";
import type { ArmingOffer as ArmingOfferData } from "../api/arming-api";
import { GlassButton } from "@/design-system";

export function ArmingOffer({
  offer,
  busy,
  error,
  onAccept,
  onDecline,
}: {
  offer: ArmingOfferData | null;
  busy: boolean;
  error: string | null;
  onAccept: () => void;
  onDecline: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const copy = useArmingOfferCopy(
    offer?.project_label ?? "",
    offer?.contributed_count ?? 0,
  );
  if (!offer && !error) return null;
  return (
    <section className="tc-card mb-4">
      <span className="mb-1.5 block tc-eyebrow">
        OPTIONAL AUTOMATION
      </span>
      {offer && (
        <>
          {copy.data ? (
            <>
              <p className="m-0 tc-label font-normal tc-text-secondary">
                {copy.data.evidence}
              </p>
              <h2>{copy.data.question}</h2>
              <p className="whitespace-pre-line text-[12px] leading-[1.55] text-tc-secondary">
                {copy.data.body}
              </p>
            </>
          ) : (
            <p className="text-[12px] text-tc-outside">
              {copy.isError
                ? "Shared arming copy unavailable. Automation is disabled."
                : "Loading automatic contribution disclosure…"}
            </p>
          )}
          <div className="mt-3 flex flex-wrap gap-2">
            <GlassButton
              type="button"
              onClick={onDecline}
              disabled={busy || !copy.data}
            >
              {copy.data?.decline ?? "Loading…"}
            </GlassButton>
            <GlassButton
              type="button"
              onClick={() => setConfirming(true)}
              disabled={busy || !copy.data}
            >
              {copy.data?.confirm ?? "Loading…"}
            </GlassButton>
          </div>
        </>
      )}
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      {offer && copy.data && (
        <ResponsiveOverlay
          open={confirming}
          onOpenChange={setConfirming}
          title={copy.data.question}
          description={copy.data.body}
          footer={
            <div className="flex justify-end gap-2">
              <GlassButton
                type="button"
                onClick={() => setConfirming(false)}
                disabled={busy}
              >
                {copy.data.decline}
              </GlassButton>
              <GlassButton
                type="button"
                onClick={() => {
                  setConfirming(false);
                  onAccept();
                }}
                disabled={busy}
              >
                {copy.data.confirm}
              </GlassButton>
            </div>
          }
        >
          <p className="m-0 tc-label font-normal tc-text-secondary">
            {copy.data.evidence}
          </p>
        </ResponsiveOverlay>
      )}
    </section>
  );
}
