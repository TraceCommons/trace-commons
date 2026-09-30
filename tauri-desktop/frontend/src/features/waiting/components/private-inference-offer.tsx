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
    <section className="tc-card mb-4">
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
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              className="tc-btn tc-btn--primary tc-btn--sm"
              type="button"
              onClick={() => onAnswer(true)}
              disabled={busy || !copy}
            >
              {copy?.offer_accept ?? "Turn it on"}
            </Button>
            <Button
              className="tc-btn tc-btn--glass"
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
        <p className="tc-alert">
          {error}
        </p>
      )}
    </section>
  );
}
