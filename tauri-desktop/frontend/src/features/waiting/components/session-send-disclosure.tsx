import { RouteDisclosureUnreadableLine } from "../../../components/route-disclosure";
import { Spinner } from "../../../components/ui/spinner";
import { routeDisclosureView } from "../../../lib/tauri/route-disclosure";
import {
  useCertificateDetail,
  useRouteDisclosure,
  useRouteDisclosureUnreadableCopy,
} from "../../../lib/tauri/use-contributor-copy";
import { useCoreStatus } from "../../../lib/tauri/use-core-status";
import type { WaitingPreview } from "../types";

// K11, for one session: what leaves this computer before redaction and after
// it, and, where a witness reviewed this session, what it was checked
// against. The sizes are the preview's own (`raw_session_bytes`,
// `would_send_bytes`); the route, the words and the certificate claims are
// the daemon's and the core's. Raw text is never shown: it does not cross
// the preview boundary.
export function SessionSendDisclosure({
  preview,
}: {
  preview: WaitingPreview;
}) {
  const core = useCoreStatus();
  const disclosure = useRouteDisclosure(core.scope, core.isSuccess);
  const certificate = useCertificateDetail(
    preview.entry.holds_certificate === true ? preview.entry.entry_id : null,
  );
  const unreadable = useRouteDisclosureUnreadableCopy();
  if (!disclosure.data) {
    // As macOS: a spinner while still being read, and the core's line --
    // or, if even that failed to load, the glyph alone -- once unreadable.
    if (routeDisclosureView(disclosure, core) === "loading") {
      return <Spinner className="size-3" />;
    }
    return (
      <RouteDisclosureUnreadableLine
        line={unreadable.data?.session}
        className="text-[11px]"
      />
    );
  }
  const { facts, copy } = disclosure.data;
  return (
    <section className="grid gap-2 border border-border px-3.5 py-3 text-[11px] leading-[1.55]">
      <span className="font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
        {copy.session.heading}
      </span>
      <dl className="m-0 grid gap-1">
        <dt className="font-bold">
          {copy.session.before_label} · {formatBytes(preview.raw_session_bytes)}
        </dt>
        <dd className="m-0">{copy.session.before_line}</dd>
        {copy.local_filter && <dd className="m-0">{copy.local_filter}</dd>}
        <dt className="font-bold">
          {copy.session.after_label} · {formatBytes(preview.would_send_bytes)}
        </dt>
        <dd className="m-0">{copy.session.after_line}</dd>
      </dl>
      {facts.route === "witness" && <p className="m-0">{copy.route}</p>}
      {certificate.data && (
        <div className="grid gap-1 rounded-md border border-border p-3">
          <strong>{certificate.data.copy.heading}</strong>
          <dl className="m-0 grid gap-1 font-mono text-[11px]">
            <dt className="text-muted-foreground">
              {certificate.data.copy.measurement_label}
            </dt>
            <dd className="m-0 break-all">
              {certificate.data.detail.witness_measurement}
            </dd>
            <dt className="text-muted-foreground">
              {certificate.data.copy.signer_label}
            </dt>
            <dd className="m-0 break-all">{certificate.data.detail.signer}</dd>
          </dl>
          <p className="m-0">{certificate.data.copy.verified_at_review}</p>
        </div>
      )}
    </section>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
