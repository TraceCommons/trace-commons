import {
  parseWitnessCapacity,
  WITNESS_SATURATED_LABEL,
  type WitnessCapacity,
} from "../../../lib/tauri/witness-capacity";
import { GATE_HELD_LABEL, parseGateHeld } from "../../../lib/tauri/switch-on-notices";
import { NEAR_AI_NOTICE_LABEL } from "../health-recovery";
import { NearAiNoticeRecovery } from "./near-ai-notice-recovery";
import { WitnessCapacityNotice } from "./witness-capacity-notice";

type Health = { last_error_label: string | null; since: string | null };
type Budget = {
  bytes_today: number;
  max_bytes_per_day: number;
  bytes_remaining: number;
  uploads_today: number;
  max_uploads_per_day: number;
  uploads_remaining: number;
  blocked: boolean;
  blocked_entries: number;
  blocked_bytes: number;
};
type Routing = {
  state: string;
  derived: boolean;
  last_refresh_at: string | null;
  unreadable_rows: number;
};

function megabytes(bytes: number) {
  return `${Math.round(bytes / 1024 / 1024)} MB`;
}
function routingLabel(state: string) {
  return (
    (
      {
        not_declared: "Not declared",
        awaiting_rows: "Waiting for proxy rows",
        rows_seen: "Receiving proxy rows",
        token_unreadable: "Proxy token unreadable",
        unknown: "Unknown",
      } as Record<string, string>
    )[state] ?? "Unknown"
  );
}

// Read apart from `health`, like the budget: the health slot holds one label
// and a higher one can mask `witness-saturated`, while this object always
// says how many sessions are waiting. A malformed object is not read as
// "none waiting" -- that would hide held sessions -- but as "unreadable".
function readCapacity(
  value: unknown,
): { kind: "none" } | { kind: "waiting"; capacity: WitnessCapacity } | { kind: "unreadable" } {
  try {
    const capacity = parseWitnessCapacity(value);
    return capacity ? { kind: "waiting", capacity } : { kind: "none" };
  } catch {
    return { kind: "unreadable" };
  }
}

function gateHeldIsDrawn(value: unknown): boolean {
  try {
    return parseGateHeld(value) !== null;
  } catch {
    // Unreadable: the app shell draws its fallback line instead.
    return true;
  }
}

export function QueueStatusPanel({
  health,
  budget,
  routing,
  witnessCapacity,
  gateHeld,
}: {
  health: Health;
  budget?: Budget;
  routing?: Routing;
  witnessCapacity?: unknown;
  gateHeld?: unknown;
}) {
  const capacity = readCapacity(witnessCapacity);
  // The held notice, drawn above every page from
  // `status.automatic_contribution_held`, says everything this label would
  // in the core's words, so the generic line steps aside for it whenever
  // that notice (or its unreadable fallback) is drawn.
  const heldShownByNotice =
    health.last_error_label === GATE_HELD_LABEL && gateHeldIsDrawn(gateHeld);
  // The capacity notice says everything this label would, in the core's
  // words, so the generic line steps aside for it -- but only when the
  // notice (or its unreadable fallback) is actually drawn.
  const saturatedShownByNotice =
    health.last_error_label === WITNESS_SATURATED_LABEL &&
    capacity.kind !== "none";
  if (
    !health.last_error_label &&
    !budget?.blocked &&
    !routing &&
    capacity.kind === "none"
  )
    return null;
  return (
    <section className="tc-card mb-4 grid gap-4">
      <div>
        <span className="mb-1.5 block tc-eyebrow">
          RUNTIME
        </span>
        <h2>Contribution safeguards</h2>
      </div>
      <div className="grid grid-cols-3 gap-px border-y border-border">
        {health.last_error_label === NEAR_AI_NOTICE_LABEL && (
          <NearAiNoticeRecovery
            label={health.last_error_label}
            since={health.since}
          />
        )}
        {capacity.kind === "waiting" && (
          <WitnessCapacityNotice capacity={capacity.capacity} />
        )}
        {capacity.kind === "unreadable" && (
          <div className="text-destructive">
            <span role="alert">
              Some approved sessions may be waiting and have not been sent,
              but this build could not read how many or why.
            </span>
          </div>
        )}
        {health.last_error_label &&
          health.last_error_label !== NEAR_AI_NOTICE_LABEL &&
          !saturatedShownByNotice &&
          !heldShownByNotice && (
          <div className="text-destructive">
            <strong>Daemon needs attention</strong>
            <span>
              Processing reported a recoverable issue. Refresh after checking
              Settings.
            </span>
            {health.since && (
              <small>Since {new Date(health.since).toLocaleString()}</small>
            )}
          </div>
        )}
        {budget && (
          <div>
            <strong>Daily limit</strong>
            <span>
              {budget.uploads_remaining} uploads left ·{" "}
              {megabytes(budget.bytes_remaining)} left
            </span>
            {budget.blocked && (
              <small>
                {budget.blocked_entries} queued session
                {budget.blocked_entries === 1 ? "" : "s"} held by limit
              </small>
            )}
          </div>
        )}
        {routing && (
          <div>
            <strong>Inference routing</strong>
            <span>
              {routingLabel(routing.state)}
              {routing.derived ? " · daemon-owned" : ""}
            </span>
            {routing.unreadable_rows > 0 && (
              <small>
                {routing.unreadable_rows} row
                {routing.unreadable_rows === 1 ? "" : "s"} unavailable
              </small>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
