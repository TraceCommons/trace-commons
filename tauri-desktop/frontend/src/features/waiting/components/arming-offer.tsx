import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import { useArmingOfferCopy } from "../../../lib/tauri/use-contributor-copy";
import type { ArmingOffer as ArmingOfferData } from "../api/arming-api";

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
    <section className="mb-4 rounded-2xl border border-border bg-card/80 p-[22px_26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        OPTIONAL AUTOMATION
      </span>
      {offer && (
        <>
          {copy.data ? (
            <>
              <p className="m-0 text-[12px] text-muted-foreground">
                {copy.data.evidence}
              </p>
              <h2>{copy.data.question}</h2>
              <p className="whitespace-pre-line text-[12px] leading-[1.55] text-muted-foreground">
                {copy.data.body}
              </p>
            </>
          ) : (
            <p className="text-[12px] text-destructive">
              {copy.isError
                ? "Shared arming copy unavailable. Automation is disabled."
                : "Loading automatic contribution disclosure…"}
            </p>
          )}
          <div className="mt-6 flex gap-2.5">
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={onDecline}
              disabled={busy || !copy.data}
            >
              {copy.data?.decline ?? "Loading…"}
            </Button>
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={() => setConfirming(true)}
              disabled={busy || !copy.data}
            >
              {copy.data?.confirm ?? "Loading…"}
            </Button>
          </div>
        </>
      )}
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
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
              <Button
                type="button"
                variant="outline"
                onClick={() => setConfirming(false)}
                disabled={busy}
              >
                {copy.data.decline}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setConfirming(false);
                  onAccept();
                }}
                disabled={busy}
              >
                {copy.data.confirm}
              </Button>
            </div>
          }
        >
          <p className="m-0 text-[12px] text-muted-foreground">
            {copy.data.evidence}
          </p>
        </ResponsiveOverlay>
      )}
    </section>
  );
}
