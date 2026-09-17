import { Button } from "@/components/ui/button";
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
  if (!offer && !error) return null;
  return (
    <section className="mb-4 rounded-2xl border border-border bg-card/80 p-[22px_26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        OPTIONAL AUTOMATION
      </span>
      {offer && (
        <>
          <h2>Keep asking about {offer.project_label}?</h2>
          <p>
            This project has contributed {offer.contributed_count} times. Enable
            automatic contribution for this project, or keep reviewing each
            session.
          </p>
          <div className="mt-6 flex gap-2.5">
            <Button
              className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
              type="button"
              onClick={onAccept}
              disabled={busy}
            >
              Enable automatic contribution
            </Button>
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={onDecline}
              disabled={busy}
            >
              Not now
            </Button>
          </div>
        </>
      )}
      {error && (
        <p className="-mt-[18px] mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}
