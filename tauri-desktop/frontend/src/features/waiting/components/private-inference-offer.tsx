import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { Button } from "@/components/ui/button";
export function PrivateInferenceOffer({
  offered,
  busy,
  error,
  onAnswer,
}: {
  offered: boolean;
  busy: boolean;
  error: string | null;
  onAnswer: (enabled: boolean) => void;
}) {
  const disclosure = useContributorDisclosureCopy();
  const copy = disclosure.data?.private_inference;
  if (!offered && !error) return null;
  return (
    <section className="mb-4 rounded-2xl border border-border bg-card/80 p-[22px_26px]">
      {copy && (
        <span className="mb-3 block font-mono text-[10px] font-extrabold uppercase leading-none tracking-[.16em] text-primary">
          {copy.destination}
        </span>
      )}
      {offered && (
        <>
          {copy && <h2>{copy.offer_title}</h2>}
          {copy ? (
            <div className="grid gap-2">
              <p className="m-0">{copy.offer_what}</p>
              <p className="m-0">{copy.offer_exposure}</p>
              <p className="m-0">{copy.offer_no_repoint}</p>
              <p className="m-0">{copy.offer_asked_once}</p>
            </div>
          ) : (
            <p className="text-destructive">
              {disclosure.isError
                ? "Disclosure unavailable. Enabling is disabled."
                : "Loading disclosure…"}
            </p>
          )}
          <div className="mt-6 flex gap-2.5">
            <Button
              className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
              type="button"
              onClick={() => onAnswer(true)}
              disabled={busy || !copy}
            >
              {copy?.offer_accept ?? "Turn it on"}
            </Button>
            <Button
              className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
              type="button"
              onClick={() => onAnswer(false)}
              disabled={busy}
            >
              {copy?.offer_decline ?? "Not now"}
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
