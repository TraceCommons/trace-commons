import type { AutomaticGrant } from "../../onboarding/public";
import { GlassButton, TertiaryLink } from "@/design-system";

// The Flow 1 grant after onboarding: whether one is in force, as the daemon
// reports it, and the way to withdraw it. Withdrawing stops new projects
// being turned on; projects the grant already turned on keep their own
// mode, which the Projects panel changes.
export function AutomaticGrantPanel({
  grant,
  state,
  error,
  withdrawn,
  onRefresh,
  onWithdraw,
  onTurnOn,
}: {
  grant: AutomaticGrant | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  withdrawn: boolean;
  onRefresh: () => Promise<void>;
  onWithdraw: () => Promise<void>;
  /** Opens the grant screens. Omitted where there is nowhere to open them. */
  onTurnOn?: () => void;
}) {
  const busy = state === "loading" || state === "busy";
  const granted = grant?.granted === true;
  return (
    <section
      className="tc-card"
      aria-labelledby="automatic-grant-heading"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            AUTOMATIC CONTRIBUTING
          </span>
          <h2 id="automatic-grant-heading">Automatic contributing</h2>
        </div>
        <TertiaryLink
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy}
        >
          Refresh
        </TertiaryLink>
      </div>
      {error && (
        <p
          className="tc-card tc-card--quiet mb-[18px] border-tc-outside/30 text-[12px] text-tc-outside"
          role="alert"
        >
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="m-0 text-[13px] text-tc-secondary" role="status">
          Reading automatic contributing…
        </p>
      )}
      {grant && (
        <div className="grid gap-3" role="status">
          {granted ? (
            <p className="m-0 text-[12px]">
              On
              {grant.granted_at
                ? ` since ${new Date(grant.granted_at).toLocaleString()}`
                : ""}
              . Projects that first appear after it was turned on contribute
              without asking.
            </p>
          ) : (
            <p className="m-0 text-[12px]">
              {withdrawn
                ? "Turned off. No automatic grant is in force."
                : "Off. No automatic grant is in force."}
            </p>
          )}
        </div>
      )}
      <p className="mt-3 mb-0 text-[11px] leading-[1.55] text-tc-secondary">
        Turning it off stops new projects from contributing automatically.
        Projects it already turned on keep their mode; change each under
        Projects.
      </p>
      {grant && !granted && onTurnOn && (
        <div className="mt-3 flex flex-wrap gap-2">
          {/* Opens the grant screens (scope, path, both disclosures); it
              turns nothing on by itself. */}
          <GlassButton
            type="button"
            onClick={onTurnOn}
            disabled={busy}
          >
            Turn on automatic contributing
          </GlassButton>
        </div>
      )}
      {granted && (
        <div className="mt-3 flex flex-wrap gap-2">
          <GlassButton
            type="button"
            onClick={() => void onWithdraw()}
            disabled={busy}
          >
            {state === "busy"
              ? "Turning off…"
              : "Turn off automatic contributing"}
          </GlassButton>
        </div>
      )}
    </section>
  );
}
