export function CreditRecordPanel({
  finalPoints,
  pendingPoints,
  refreshedAt,
}: {
  finalPoints: number;
  pendingPoints: number;
  refreshedAt: string | null;
}) {
  return (
    <section className="mt-4 grid grid-cols-[64px_minmax(0,1fr)] gap-5 p-[26px] rounded-2xl border border-border bg-card/80">
      <div
        className="mt-[3px] ml-[3px] h-[58px] w-[58px] rounded-full border-2 border-foreground bg-primary/20 shadow-md"
        aria-hidden="true"
      />
      <div className="grid gap-2.5">
        <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          CREDIT RECORD
        </span>
        <h2>About credit.</h2>
        <p>
          Contributions earn credit points, scored on novelty and information
          richness. Today credit is a record, not currency: no payout, token,
          exchange rate, or date.
        </p>
        {refreshedAt ? (
          <div className="mt-1 flex flex-wrap gap-8 border-t border-border pt-3.5">
            <CreditFigure label="Final" value={finalPoints} />
            <CreditFigure label="Still being scored" value={pendingPoints} />
          </div>
        ) : (
          <span className="whitespace-nowrap rounded-full bg-primary/10 px-2.5 py-[7px] font-mono text-[10px] font-extrabold tracking-[.08em] text-primary max-[860px]:col-start-2 max-[860px]:justify-self-start bg-muted text-muted-foreground">
            Not synced yet
          </span>
        )}
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          A credit is a signed record that a contribution was accepted. It is
          not currency.
        </p>
      </div>
    </section>
  );
}

function CreditFigure({ label, value }: { label: string; value: number }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value.toFixed(1)}</strong>
    </div>
  );
}
