import { Button } from "@/components/ui/button";
import type { CoreStatus } from "../../../lib/tauri/types";
import type { RoutingDiscovery, RoutingEvidence } from "../api/routing-api";
import { RoutingControls } from "./routing-controls";

function stateLabel(state: string) {
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

export function RoutingPanel({
  snapshot,
  status,
  discovery,
  evidence,
  state,
  error,
  onRefresh,
  onCheck,
  onConfigure,
}: {
  snapshot: Record<string, unknown>;
  status: CoreStatus | null;
  discovery: RoutingDiscovery | null;
  evidence: RoutingEvidence | null;
  state: "loading" | "ready" | "busy" | "error";
  error: string | null;
  onRefresh: () => Promise<void>;
  onCheck: (port: number, tokenDir: string) => Promise<unknown>;
  onConfigure: (
    enabled: boolean,
    port: number,
    tokenDir: string,
  ) => Promise<unknown>;
}) {
  const declaration =
    typeof snapshot.ironwire === "object" && snapshot.ironwire !== null
      ? (snapshot.ironwire as Record<string, unknown>)
      : null;
  const declarationMode =
    typeof declaration?.mode === "string" ? declaration.mode : null;
  const declarationPort =
    typeof declaration?.port === "number" ? declaration.port : null;
  const declarationTokenDir =
    typeof declaration?.token_dir === "string" ? declaration.token_dir : "";
  const busy = state === "loading" || state === "busy";
  return (
    <section className="tc-card block">
      <div className="flex items-start justify-between gap-3">
        <div>
          <span className="mb-1.5 block tc-eyebrow">
            PRIVATE ROUTING
          </span>
          <h2>Local proxy boundary</h2>
        </div>
        <Button
          className="tc-link"
          type="button"
          onClick={() => void onRefresh()}
          disabled={busy}
        >
          Discover
        </Button>
      </div>
      <p className="m-0 tc-caption tc-text-tertiary">
        Routing is opt-in. The daemon reads only a declared local proxy and
        never sends its token to the UI. A declaration does not prove that any
        agent is connected.
      </p>
      {error && (
        <p className="tc-alert">
          {error}
        </p>
      )}
      <div className="tc-card tc-card--quiet mt-5 grid gap-[5px]">
        <strong>
          {stateLabel(status?.daemon.routing?.state ?? "unknown")}
        </strong>
        <span>
          {status?.daemon.routing?.derived
            ? "Derived from daemon-owned routing rows."
            : "No live routing rows available."}
        </span>
      </div>
      {discovery?.found && (
        <p className="mt-3.5 text-[11px] leading-[1.45] text-muted-foreground">
          Proxy discovery found port {discovery.port ?? "unknown"}
          {discovery.token_path
            ? ` · token directory ${discovery.token_path}`
            : ""}
          . Review values before saving.
        </p>
      )}
      <RoutingControls
        enabled={declarationMode === "watch"}
        port={Number(declarationPort ?? discovery?.port ?? 8463)}
        tokenDir={declarationTokenDir}
        busy={busy}
        evidence={evidence}
        onCheck={onCheck}
        onConfigure={onConfigure}
      />
    </section>
  );
}
