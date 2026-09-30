import type { RouteDisclosure } from "../lib/tauri/route-disclosure";
import { useRouteDisclosure } from "../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../lib/tauri/use-core-status";

// K11: the raw send, both enclaves, and where the witness came from. Every
// fact is the daemon's (`route_disclosure`) and every sentence is the
// contributor core's; this file only lays them out. A block is drawn only
// when the core sent words for it, and the parser has already refused words
// that do not match the facts.

/** The disclosure itself, for a surface that has already loaded it. */
export function RouteDisclosureBody({
  disclosure,
}: {
  disclosure: RouteDisclosure;
}) {
  const { facts, copy } = disclosure;
  return (
    <div className="grid gap-3 text-[12px] leading-[1.55]">
      <p className="m-0">{copy.route}</p>
      {copy.local_filter && <p className="m-0">{copy.local_filter}</p>}
      {facts.witness && copy.witness && (
        <div className="tc-card tc-card--quiet grid gap-2">
          <strong className="text-[12px]">{copy.witness.heading}</strong>
          <dl className="m-0 grid gap-1 font-mono text-[11px]">
            <dt className="text-tc-secondary">
              {copy.witness.address_label}
            </dt>
            <dd className="m-0 break-all">{facts.witness.url}</dd>
            <dt className="text-tc-secondary">
              {copy.witness.signing_label}
            </dt>
            <dd className="m-0 break-all">{facts.witness.signing_address}</dd>
            <dt className="text-tc-secondary">
              {copy.witness.measurements_label}
            </dt>
            {facts.witness.pinned_measurements.map((pin) => (
              <dd className="m-0 break-all" key={pin}>
                {pin}
              </dd>
            ))}
          </dl>
          <p className="m-0">{copy.witness.check}</p>
          {copy.witness.classifier && (
            <p className="m-0">{copy.witness.classifier}</p>
          )}
          <p className="m-0">{copy.witness.origin}</p>
        </div>
      )}
      {copy.attested_bodies && <p className="m-0">{copy.attested_bodies}</p>}
      {copy.receipts && <p className="m-0">{copy.receipts}</p>}
    </div>
  );
}

/**
 * A settings section that reads the disclosure itself. Unreadable is said as
 * such and never drawn as some other route.
 */
export function RouteDisclosurePanel() {
  const core = useCoreStatus();
  const disclosure = useRouteDisclosure(core.scope, core.isSuccess);
  return (
    <section className="tc-card">
      <span className="mb-1.5 block tc-eyebrow">
        WHERE SESSIONS GO
      </span>
      {disclosure.data ? (
        <>
          <h2>{disclosure.data.copy.title}</h2>
          <RouteDisclosureBody disclosure={disclosure.data} />
        </>
      ) : (
        <p
          className={`m-0 text-[12px] ${disclosure.isError ? "text-tc-outside" : "text-tc-secondary"}`}
          role={disclosure.isError ? "alert" : "status"}
        >
          {disclosure.isError
            ? "Where sessions go could not be read."
            : "Reading where sessions go…"}
        </p>
      )}
    </section>
  );
}
