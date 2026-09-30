import { WarningIcon } from "@phosphor-icons/react";
import {
  type RouteDisclosure,
  routeDisclosureView,
} from "../lib/tauri/route-disclosure";
import {
  useRouteDisclosure,
  useRouteDisclosureUnreadableCopy,
} from "../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../lib/tauri/use-core-status";
import { Spinner } from "./ui/spinner";

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
 * The unreadable state. Marked by a glyph as well as colour, and drawn even
 * when the core's sentence for it failed to load, so the section is never
 * simply empty -- as macOS's `RouteDisclosureUnreadableLine`.
 */
export function RouteDisclosureUnreadableLine({
  line,
  className,
}: {
  line: string | undefined;
  className: string;
}) {
  return (
    <p
      className={`m-0 flex items-baseline gap-2 text-destructive ${className}`}
      role="alert"
    >
      <WarningIcon
        className="shrink-0 self-center"
        aria-hidden={line !== undefined}
      />
      {line}
    </p>
  );
}

/**
 * A settings section that reads the disclosure itself. Unreadable is said as
 * such and never drawn as some other route. The title and the unreadable
 * line are the core's (`route_disclosure_unreadable_copy`); a spinner shows
 * while the disclosure is still being read, as on macOS.
 */
export function RouteDisclosurePanel() {
  const core = useCoreStatus();
  const disclosure = useRouteDisclosure(core.scope, core.isSuccess);
  const unreadable = useRouteDisclosureUnreadableCopy();
  const view = routeDisclosureView(disclosure, core);
  const title = disclosure.data?.copy.title ?? unreadable.data?.title;
  return (
    <section className="rounded-2xl border border-border bg-card/80 p-[26px]">
      {title && (
        <h2 className="m-0 mb-3 block font-mono text-[10px] font-extrabold uppercase leading-none tracking-[.16em] text-primary">
          {title}
        </h2>
      )}
      {view === "shown" && disclosure.data && (
        <RouteDisclosureBody disclosure={disclosure.data} />
      )}
      {view === "loading" && <Spinner className="size-3" />}
      {view === "unreadable" && (
        <RouteDisclosureUnreadableLine
          line={unreadable.data?.panel}
          className="text-[12px]"
        />
      )}
    </section>
  );
}
