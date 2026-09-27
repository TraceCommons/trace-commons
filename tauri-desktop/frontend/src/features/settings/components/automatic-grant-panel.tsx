import { Button } from "@/components/ui/button";
import type { AutomaticGrant } from "../../onboarding/public";

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
}: {
  grant: AutomaticGrant | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  withdrawn: boolean;
  onRefresh: () => Promise<void>;
  onWithdraw: () => Promise<void>;
}) {
  const busy = state === "loading" || state === "busy";
  const granted = grant?.granted === true;
  return (
    <section
      className="rounded-2xl border border-border bg-card/80 p-[26px]"
      aria-labelledby="automatic-grant-heading"
    >
      <div className="flex items-start justify-between gap-[18px]">
        <div>
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            AUTOMATIC CONTRIBUTING
          </span>
          <h2 id="automatic-grant-heading">Automatic contributing</h2>
        </div>
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy}
        >
          Refresh
        </Button>
      </div>
      {error && (
        <p
          className="mb-[18px] rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive"
          role="alert"
        >
          {error}
        </p>
      )}
      {state === "loading" && (
        <p className="m-0 text-[13px] text-muted-foreground" role="status">
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
      <p className="mt-3 mb-0 text-[11px] leading-[1.55] text-muted-foreground">
        Turning it off stops new projects from contributing automatically.
        Projects it already turned on keep their mode; change each under
        Projects.
      </p>
      {granted && (
        <div className="mt-6 flex gap-2.5">
          <Button
            type="button"
            variant="outline"
            onClick={() => void onWithdraw()}
            disabled={busy}
          >
            {state === "busy"
              ? "Turning off…"
              : "Turn off automatic contributing"}
          </Button>
        </div>
      )}
    </section>
  );
}
