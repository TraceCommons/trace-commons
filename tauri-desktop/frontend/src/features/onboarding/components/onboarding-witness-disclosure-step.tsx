import { Button } from "@/components/ui/button";
import { RouteDisclosureBody } from "../../../components/route-disclosure";
import { useRouteDisclosure } from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import { useWitness } from "../../settings/public";
import type { OnboardingStepProps } from "./onboarding-step-types";

// K11, second screen: whether a session leaves this machine unredacted, for
// whom (both enclaves), and where the witness came from. Every fact is the
// daemon's (`route_disclosure`) or the witness status's, not assumed; every
// sentence is the core's.
export function OnboardingWitnessDisclosureStep({
  onboarding,
  busy,
}: Pick<OnboardingStepProps, "onboarding" | "busy">) {
  const core = useCoreStatus();
  const disclosure = useRouteDisclosure(core.scope, core.isSuccess);
  const witness = useWitness();
  const status = witness.data;
  // Only a pinned witness is sent sessions; a refusing one sends nothing.
  const rawSend = status?.state === "pinned";
  // The screen records the witness it showed, so the two reads must agree
  // on which witness that is before Continue means anything.
  const agree =
    (disclosure.data?.facts.witness?.signing_address ?? null) ===
      (status?.signing_address ?? null) &&
    (disclosure.data?.facts.route === "witness") === rawSend;
  const ready = Boolean(
    status && disclosure.data && witness.state === "ready" && agree,
  );
  const failed =
    witness.state === "error" ||
    disclosure.isError ||
    Boolean(disclosure.data && status && !agree);
  return (
    <section className="rounded-2xl border border-border bg-card/80 mb-4 p-[26px]">
      <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        WHERE SESSIONS GO
      </span>
      <h2>Redaction witness</h2>
      {ready && status && disclosure.data ? (
        <div className="grid gap-3">
          <p className="m-0" role="status">
            {status.state_line}
          </p>
          <RouteDisclosureBody disclosure={disclosure.data} />
        </div>
      ) : (
        <p
          className={`m-0 text-[12px] ${failed ? "text-destructive" : "text-muted-foreground"}`}
          role={failed ? "alert" : "status"}
        >
          {failed
            ? "Witness status could not be read. Continue is disabled."
            : "Reading witness status…"}
        </p>
      )}
      <div className="mt-6 flex gap-2.5">
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={onboarding.back}
          disabled={busy}
        >
          Back
        </Button>
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={() =>
            onboarding.acknowledgeWitnessDisclosure(
              status?.signing_address ?? null,
            )
          }
          disabled={busy || !ready}
        >
          Continue
        </Button>
      </div>
    </section>
  );
}
