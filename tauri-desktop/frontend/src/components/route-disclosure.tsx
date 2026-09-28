import type { RouteDisclosure } from "../lib/tauri/route-disclosure";
import {
  useRouteDisclosure,
  useRouteDisclosureUnreadableCopy,
} from "../lib/tauri/use-contributor-copy";
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
        <div className="grid gap-2 rounded-md border border-border p-3">
          <strong className="text-[12px]">{copy.witness.heading}</strong>
          <dl className="m-0 grid gap-1 font-mono text-[11px]">
            <dt className="text-muted-foreground">
              {copy.witness.address_label}
            </dt>
            <dd className="m-0 break-all">{facts.witness.url}</dd>
            <dt className="text-muted-foreground">
              {copy.witness.signing_label}
            </dt>
            <dd className="m-0 break-all">{facts.witness.signing_address}</dd>
            <dt className="text-muted-foreground">
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
 * such and never drawn as some other route. The title and the unreadable
 * line are the core's (`route_disclosure_unreadable_copy`); nothing is said
 * while the disclosure is still being read.
 */
export function RouteDisclosurePanel() {
  const core = useCoreStatus();
  const disclosure = useRouteDisclosure(core.scope, core.isSuccess);
  const unreadable = useRouteDisclosureUnreadableCopy();
  const title = disclosure.data?.copy.title ?? unreadable.data?.title;
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      {title && (
        <h2 className="m-0 mb-3 block font-mono text-[10px] font-extrabold uppercase leading-none tracking-[.16em] text-primary">
          {title}
        </h2>
      )}
      {disclosure.data ? (
        <RouteDisclosureBody disclosure={disclosure.data} />
      ) : (
        disclosure.isError &&
        unreadable.data && (
          <p className="m-0 text-[12px] text-destructive" role="alert">
            {unreadable.data.panel}
          </p>
        )
      )}
    </section>
  );
}
