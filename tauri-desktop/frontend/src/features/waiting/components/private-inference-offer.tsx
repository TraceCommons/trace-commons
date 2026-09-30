import { useContributorDisclosureCopy } from "../../../lib/tauri/use-contributor-copy";
import { ButtonPrimary, GlassButton } from "@/design-system";
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
        <span className="mb-3 block font-mono text-[10px] font-extrabold uppercase leading-none tracking-[.16em] text-tc-accent">
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
            <p className="text-tc-outside">
              {disclosure.isError
                ? "Disclosure unavailable. Enabling is disabled."
                : "Loading disclosure…"}
            </p>
          )}
          <div className="mt-3 flex flex-wrap gap-2">
            <ButtonPrimary size="sm"
              type="button"
              onClick={() => onAnswer(true)}
              disabled={busy || !copy}
            >
              {copy?.offer_accept ?? "Turn it on"}
            </ButtonPrimary>
            <GlassButton
              type="button"
              onClick={() => onAnswer(false)}
              disabled={busy}
            >
              {copy?.offer_decline ?? "Not now"}
            </GlassButton>
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
