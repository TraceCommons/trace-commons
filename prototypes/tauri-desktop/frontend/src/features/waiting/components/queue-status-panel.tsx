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

export function QueueStatusPanel({
  health,
  budget,
  routing,
}: {
  health: Health;
  budget?: Budget;
  routing?: Routing;
}) {
  if (!health.last_error_label && !budget?.blocked && !routing) return null;
  return (
    <section className="mb-4 grid gap-4 rounded-[14px] border border-border bg-white/[.68] p-[22px]">
      <div>
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          RUNTIME
        </span>
        <h2>Contribution safeguards</h2>
      </div>
      <div className="grid grid-cols-3 gap-px border-y border-border">
        {health.last_error_label && (
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
