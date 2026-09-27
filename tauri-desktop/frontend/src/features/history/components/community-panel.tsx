import type { CommunityStanding } from "../types";

export function CommunityPanel({ standing }: { standing: CommunityStanding }) {
  const rate =
    standing.accept_rate === null
      ? "—"
      : `${Math.round(standing.accept_rate * 100)}%`;
  const snapshot = standing.snapshot_at
    ? new Date(standing.snapshot_at).toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
      })
    : "date unavailable";
  return (
    <section className="mb-4 border-2 border-foreground bg-primary/10 p-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <span>COMMUNITY</span>
          <h2>Public standing</h2>
        </div>
        <span className="grid h-[34px] w-[34px] place-items-center bg-primary text-[11px] font-extrabold text-primary-foreground">
          TC
        </span>
      </div>
      <div className="mt-[22px] grid grid-cols-4 border-y border-primary/30">
        <div>
          <span>Rank</span>
          <strong>{standing.rank === null ? "—" : `#${standing.rank}`}</strong>
        </div>
        <div>
          <span>Novelty credit</span>
          <strong>{standing.novelty_credit}</strong>
        </div>
        <div>
          <span>Accepted · {standing.window_label}</span>
          <strong>{standing.accepted_in_window}</strong>
        </div>
        <div>
          <span>Accept rate</span>
          <strong>{rate}</strong>
        </div>
      </div>
      <p>
        Shown only while your public handle is active. Snapshot: {snapshot}.
      </p>
      {standing.analytics_withheld && (
        <p className="font-bold">Aggregate analytics withheld by policy.</p>
      )}
    </section>
  );
}
