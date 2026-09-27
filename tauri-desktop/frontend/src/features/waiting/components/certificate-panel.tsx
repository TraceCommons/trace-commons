import type { CertificateCopy, WaitingEntry } from "../types";

export function CertificatePanel({
  entries,
  copy,
}: {
  entries: WaitingEntry[];
  copy: CertificateCopy | null;
}) {
  const certified = entries.filter((entry) => entry.holds_certificate === true);
  return (
    <section className="mb-4 grid gap-2.5 rounded-xl border border-chart-4/30 bg-chart-4/10 p-[22px_26px]">
      <div>
        <h2>{copy?.list_title ?? "Witness certificates"}</h2>
      </div>
      {certified.length > 0 ? (
        <div className="mt-1 grid gap-3 border-t border-chart-4/20 pt-3">
          {certified.map((entry) => (
            <div key={entry.entry_id} className="grid gap-1">
              <strong>{entry.project_label}</strong>
              <span>{copy?.row_line ?? "Witness certificate held."}</span>
            </div>
          ))}
        </div>
      ) : (
        <p>{copy?.list_empty ?? "Certificate list copy unavailable."}</p>
      )}
    </section>
  );
}
