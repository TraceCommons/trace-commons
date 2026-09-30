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
    <section className="mt-4 grid grid-cols-[64px_minmax(0,1fr)] gap-5 tc-card">
      <div
        className="tc-card mt-[3px] ml-[3px] h-[58px] w-[58px]"
        aria-hidden="true"
      />
      <div className="grid gap-2.5">
        <span className="mb-1.5 block tc-eyebrow">
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
          <span className="tc-chip self-start">
            Not synced yet
          </span>
        )}
        <p className="m-0 tc-caption tc-text-tertiary">
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
